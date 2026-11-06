//! UI-independent bridge. Secrets stay on the worker, never in snapshots/settings.
use porch_core::{Api, Engine, GatewayTask, PendingSend, RemovalNotice};
use porch_protocol::*;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
pub struct Snapshot {
    pub generation: u64,
    pub user: Option<User>,
    pub state: String,
    pub error: Option<String>,
    pub heads: Vec<ConversationHead>,
    pub communities: Vec<CommunitySnapshot>,
    pub selected: Option<i64>,
    pub messages: Vec<Message>,
    pub pending: Vec<PendingSend>,
    pub notice: Option<RemovalNotice>,
    /// Only a durable queue acknowledgement may clear the UI composer.
    pub queued: Option<u64>,
    pub edited: Option<u64>,
    pub busy: bool,
}
pub enum Command {
    Login {
        origin: String,
        username: String,
        password: String,
        invite: Option<String>,
    },
    Select(i64),
    Send {
        id: u64,
        conversation: i64,
        text: String,
        reply_to: Option<i64>,
    },
    Mutate {
        conversation: i64,
        message: i64,
        command: MessageCommand,
    },
    Edit {
        id: u64,
        conversation: i64,
        message: i64,
        text: String,
    },
    Retry(String),
    Discard(String),
    Older,
    CreateDm(Vec<i64>),
    Refresh,
    Logout,
    AcknowledgeNotice,
}
pub struct Bridge {
    pub commands: mpsc::Sender<Command>,
    pub latest: Arc<Mutex<Arc<Snapshot>>>,
    pub changes: async_channel::Receiver<()>,
}
pub fn data_dir() -> std::io::Result<PathBuf> {
    let root = if let Some(p) = std::env::var_os("PORCH_DATA_DIR") {
        PathBuf::from(p)
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or(std::io::ErrorKind::NotFound)?)
            .join("Porch")
    } else if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(p).join("porch")
    } else {
        PathBuf::from(std::env::var_os("HOME").ok_or(std::io::ErrorKind::NotFound)?)
            .join(".local/share/porch")
    };
    std::fs::create_dir_all(&root)?;
    if std::fs::symlink_metadata(&root)?.file_type().is_symlink() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(root)
}
fn cache_path(root: &Path, origin: &str, user: i64) -> PathBuf {
    let digest = Sha256::digest(format!("{origin}\0{user}").as_bytes());
    root.join(format!("{digest:x}.sqlite3"))
}
impl Bridge {
    pub fn start(root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let (changed, changes) = async_channel::bounded(1);
        let latest = Arc::new(Mutex::new(Arc::new(Snapshot {
            state: "Signed out".into(),
            ..Default::default()
        })));
        let output = latest.clone();
        std::thread::Builder::new()
            .name("porch-session".into())
            .spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("start session runtime");
                runtime.block_on(worker(root, rx, output, changed));
            })
            .expect("start session worker");
        Self {
            commands: tx,
            latest,
            changes,
        }
    }
}
struct Session {
    root: PathBuf,
    engine: Option<Engine>,
    gateway: Option<GatewayTask>,
    view: Snapshot,
    before: Option<i64>,
}
impl Session {
    async fn login(
        &mut self,
        origin: String,
        username: String,
        password: String,
        invite: Option<String>,
    ) -> porch_core::Result<()> {
        self.gateway.take();
        self.engine.take();
        self.view = Snapshot {
            generation: self.view.generation + 1,
            state: "Signing in".into(),
            busy: true,
            ..Default::default()
        };
        let api = Api::new(&origin, "login".into())?;
        let response: AuthResponse = if let Some(code) = invite {
            api.post(
                "/auth/register",
                &RegisterRequest {
                    username,
                    password,
                    invite_code: code,
                },
            )
            .await?
        } else {
            api.post("/auth/login", &LoginRequest { username, password })
                .await?
        };
        self.connect(api.origin(), response.token, response.user.id)
            .await
    }
    async fn connect(&mut self, origin: &str, token: String, user: i64) -> porch_core::Result<()> {
        self.before = None;
        let path = cache_path(&self.root, origin, user);
        let engine = Engine::open(origin, token, path).await?;
        self.view.user = Some(engine.user().clone());
        self.gateway = Some(engine.start_gateway()?);
        self.engine = Some(engine.clone());
        // Transport failure still leaves an authenticated last-known offline cache.
        engine.reconcile().await?;
        self.view.selected = engine.cache().heads().await?.first().map(|h| h.id);
        Ok(())
    }
    async fn refresh(&mut self) -> porch_core::Result<()> {
        let Some(engine) = self.engine.clone() else {
            return Ok(());
        };
        if engine.is_signed_out() {
            let notice = engine.cache().removal_notice().await?;
            self.gateway.take();
            self.engine.take();
            self.view=Snapshot{generation:self.view.generation+1,notice,error:Some("Session ended. Local drafts were removed; some may already have been delivered.".into()),state:"Signed out".into(),..Default::default()};
            return Ok(());
        }
        self.view.user = Some(engine.user().clone());
        self.view.heads = engine.cache().heads().await?;
        self.view.communities = engine.cache().communities().await?;
        self.view.pending = engine.cache().pending().await?;
        self.view.notice = engine.cache().removal_notice().await?;
        if self
            .view
            .selected
            .is_some_and(|id| !self.view.heads.iter().any(|h| h.id == id))
        {
            self.view.selected = None;
        }
        if let Some(id) = self.view.selected {
            self.view.messages = engine.cache().messages(id, self.before, 200).await?;
            self.view.messages.reverse();
        } else {
            self.view.messages.clear();
        }
        self.view.state = self
            .gateway
            .as_ref()
            .map(|g| format!("{:?}", *g.state().borrow()))
            .unwrap_or_else(|| "Offline".into());
        Ok(())
    }
    async fn command(&mut self, command: Command) -> porch_core::Result<()> {
        if let Command::Login {
            origin,
            username,
            password,
            invite,
        } = command
        {
            return self.login(origin, username, password, invite).await;
        }
        let engine = self.engine.clone().ok_or(porch_core::Error::InvalidData)?;
        match command {
            Command::Select(id) => {
                if !self.view.heads.iter().any(|h| h.id == id) {
                    return Err(porch_core::Error::InvalidData);
                }
                self.view.selected = Some(id);
                self.before = None;
                self.view.messages = engine.cache().messages(id, self.before, 200).await?;
                // Select is the deliberate read action, not an automatic background read.
                if let Some(last) = self
                    .view
                    .messages
                    .iter()
                    .filter(|m| !m.deleted)
                    .map(|m| m.id)
                    .max()
                {
                    engine
                        .api()
                        .post_empty(
                            &format!("/conversations/{id}/read"),
                            &MarkRead { message_id: last },
                        )
                        .await?;
                }
            }
            Command::Send {
                id,
                conversation,
                text,
                reply_to,
            } => {
                engine
                    .queue(
                        conversation,
                        MessageContent::Markdown { text },
                        reply_to,
                        vec![],
                        false,
                    )
                    .await?;
                self.view.queued = Some(id);
                tokio::spawn(async move {
                    let _ = engine.flush().await;
                });
            }
            Command::Edit {
                id,
                conversation,
                message,
                text,
            } => {
                engine
                    .mutate(
                        conversation,
                        message,
                        &MessageCommand::Edit {
                            content: MessageContent::Markdown { text },
                            mentions: vec![],
                            mention_everyone: false,
                        },
                    )
                    .await?;
                self.view.edited = Some(id);
            }
            Command::Mutate {
                conversation,
                message,
                command,
            } => {
                engine.mutate(conversation, message, &command).await?;
            }
            Command::Retry(nonce) => {
                engine.retry(&nonce).await?;
                tokio::spawn(async move {
                    let _ = engine.flush().await;
                });
            }
            Command::Discard(nonce) => engine.discard(&nonce).await?,
            Command::Older => {
                if let Some(id) = self.view.selected {
                    let before = self.view.messages.iter().map(|m| m.id).min();
                    engine.older(id, before).await?;
                    self.before = before;
                }
            }
            Command::CreateDm(participants) => {
                let head: ConversationHead = engine
                    .api()
                    .post(
                        "/conversations",
                        &CreateDm {
                            participants,
                            name: None,
                        },
                    )
                    .await?;
                engine.reconcile().await?;
                self.before = None;
                self.view.selected = Some(head.id);
            }
            Command::Refresh => {
                engine.flush().await?;
            }
            Command::Logout => {
                self.gateway.take();
                let result = engine.logout().await;
                let notice = engine.cache().removal_notice().await?;
                self.engine.take();
                self.view = Snapshot {
                    generation: self.view.generation + 1,
                    notice,
                    state: "Signed out".into(),
                    ..Default::default()
                };
                result?;
            }
            Command::AcknowledgeNotice => {
                engine.cache().acknowledge_removal_notice().await?;
                self.view.notice = None;
            }
            Command::Login { .. } => unreachable!(),
        }
        Ok(())
    }
}
fn publish(view: &Snapshot, output: &Mutex<Arc<Snapshot>>, changed: &async_channel::Sender<()>) {
    *output.lock().expect("snapshot lock") = Arc::new(view.clone());
    let _ = changed.try_send(());
}
async fn worker(
    root: PathBuf,
    mut commands: mpsc::Receiver<Command>,
    output: Arc<Mutex<Arc<Snapshot>>>,
    changed: async_channel::Sender<()>,
) {
    let mut session = Session {
        root,
        engine: None,
        gateway: None,
        before: None,
        view: Snapshot {
            state: "Signed out".into(),
            ..Default::default()
        },
    };
    // Environment login is for development/smoke, never a command-line secret.
    if let (Ok(origin), Ok(token)) = (std::env::var("PORCH_URL"), std::env::var("PORCH_TOKEN")) {
        let result = async {
            let api = Api::new(&origin, token.clone())?;
            let user: User = api.get("/auth/me").await?;
            session.connect(api.origin(), token, user.id).await
        }
        .await;
        if let Err(e) = result {
            session.view.error = Some(e.to_string());
        }
    }
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            command=commands.recv()=>{
                let Some(command)=command else{break};
                session.view.busy=true;session.view.error=None;
                publish(&session.view,&output,&changed);
                if let Err(e)=session.command(command).await{session.view.error=Some(e.to_string());}
                session.view.busy=false;
            },
            _=tick.tick()=>{}
        }
        if let Err(e) = session.refresh().await {
            session.view.error = Some(e.to_string());
        }
        publish(&session.view, &output, &changed);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_cache_names_are_stable_and_disjoint() {
        let root = Path::new("/private");
        assert_eq!(
            cache_path(root, "https://porch.test", 1),
            cache_path(root, "https://porch.test", 1)
        );
        assert_ne!(
            cache_path(root, "https://porch.test", 1),
            cache_path(root, "https://porch.test", 2)
        );
        assert_ne!(
            cache_path(root, "https://porch.test", 1),
            cache_path(root, "https://other.test", 1)
        );
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    use porch_server::{
        MIGRATOR,
        communities::{self, CommunityService},
        gateway::{EventBus, Gateway},
        identity::{self, IdentityState},
        messaging::MessagingService,
    };
    use std::time::Duration;

    async fn receive(bridge: &Bridge, predicate: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let current = bridge.latest.lock().unwrap().clone();
                if predicate(&current) {
                    return current;
                }
                bridge.changes.recv().await.unwrap();
            }
        })
        .await
        .expect("desktop session snapshot deadline")
    }
    #[tokio::test]
    #[ignore = "disposable Postgres; CI runs explicitly"]
    async fn postgres_desktop_session_commands() {
        let url = std::env::var("DATABASE_URL").unwrap();
        let db = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("desktop_{}", rand::random::<u64>());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&db)
            .await
            .unwrap();
        let search = schema.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .after_connect(move |conn, _| {
                let q = format!("SET search_path TO {search}");
                Box::pin(async move {
                    sqlx::query(&q).execute(conn).await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let user = identity::bootstrap(
            &pool,
            "desktop_admin".into(),
            "desktop long password".into(),
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES(99,'desktop_peer')")
            .execute(&pool)
            .await
            .unwrap();
        communities::initialize(&pool).await.unwrap();
        let identity = IdentityState::new(pool.clone()).await.unwrap();
        let gateway = Gateway::new(identity.clone());
        let bus: Arc<dyn EventBus> = Arc::new(gateway.clone());
        identity.set_event_bus(&bus);
        let app = CommunityService::new(pool.clone(), identity.clone(), bus.clone())
            .routes()
            .merge(MessagingService::new(pool.clone(), identity.clone(), bus).routes())
            .merge(identity::routes(identity))
            .merge(gateway.routes());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let bridge = Bridge::start(root.path().to_owned());
        bridge
            .commands
            .send(Command::Login {
                origin,
                username: "desktop_admin".into(),
                password: "desktop long password".into(),
                invite: None,
            })
            .await
            .unwrap();
        let signed = receive(&bridge, |s| s.user.is_some() && !s.busy).await;
        assert_eq!(signed.user.as_ref().unwrap().id, user.id);
        assert!(signed.error.is_none(), "{:?}", signed.error);
        bridge
            .commands
            .send(Command::CreateDm(vec![99]))
            .await
            .unwrap();
        let dm = receive(&bridge, |s| {
            s.heads
                .iter()
                .any(|h| Some(h.id) == s.selected && h.community_id.is_none())
                && !s.busy
        })
        .await
        .selected
        .unwrap();
        bridge
            .commands
            .send(Command::Send {
                id: 1,
                conversation: dm,
                text: "desktop durable hello".into(),
                reply_to: None,
            })
            .await
            .unwrap();
        let queued = receive(&bridge, |s| s.queued == Some(1)).await;
        assert!(queued.error.is_none());
        let sent = receive(&bridge, |s| {
            s.messages.iter().any(|m| m.author_id == user.id)
        })
        .await;
        let message = sent.messages.last().unwrap().id;
        bridge
            .commands
            .send(Command::Edit {
                id: 2,
                conversation: dm,
                message,
                text: "desktop edited hello".into(),
            })
            .await
            .unwrap();
        let edited = receive(&bridge, |s| s.edited == Some(2) && !s.busy).await;
        assert!(edited.error.is_none());
        assert!(
            matches!(&edited.messages.last().unwrap().content,Some(MessageContent::Markdown{text}) if text=="desktop edited hello")
        );
        bridge
            .commands
            .send(Command::Edit {
                id: 3,
                conversation: dm,
                message,
                text: "".into(),
            })
            .await
            .unwrap();
        let failed = receive(&bridge, |s| s.error.is_some() && !s.busy).await;
        assert_eq!(
            failed.edited,
            Some(2),
            "failed edit must not acknowledge/clear composer"
        );
        bridge.commands.send(Command::Select(999999)).await.unwrap();
        let invalid = receive(&bridge, |s| s.error.is_some() && !s.busy).await;
        assert_eq!(invalid.selected, Some(dm));
        bridge.commands.send(Command::Logout).await.unwrap();
        let logout = receive(&bridge, |s| {
            s.user.is_none() && s.generation > signed.generation && !s.busy
        })
        .await;
        assert!(logout.heads.is_empty() && logout.messages.is_empty() && logout.pending.is_empty());
        drop(bridge);
        server.abort();
        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&db)
            .await
            .unwrap();
        db.close().await;
    }
}
