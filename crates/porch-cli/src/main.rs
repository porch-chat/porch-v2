mod files;
use porch_protocol::*;
use reqwest::{
    Method,
    blocking::{Client, Response},
};
use serde::Serialize;
use std::{env, error::Error, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Cli {
    client: Client,
    base: String,
}

impl Cli {
    fn request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
        auth: bool,
    ) -> Result<Response> {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if auth {
            request = request.bearer_auth(env::var("PORCH_TOKEN")?);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send()?;
        if !response.status().is_success() {
            // Do not echo request credentials or raw server text.
            return Err(format!("request failed with HTTP {}", response.status()).into());
        }
        Ok(response)
    }

    fn read<T: serde::de::DeserializeOwned + Serialize>(&self, path: &str) -> Result<()> {
        let response: T = self.request::<()>(Method::GET, path, None, true)?.json()?;
        print_json(&response)
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.request::<()>(Method::DELETE, path, None, true)?;
        println!("ok");
        Ok(())
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return Err("usage: porch-cli communities|community ID|community-create NAME|community-join|community-mutate ID|community-invite-create ID USES SECONDS|community-invites ID|community-invite-revoke ID INVITE_ID|register|login|me|sessions|logout|logout-all|revoke-session ID|invite-create USES SECONDS|invites|invite-revoke ID; credentials from PORCH_USERNAME/PORCH_PASSWORD/PORCH_INVITE/PORCH_TOKEN".into());
    };
    let base = env::var("PORCH_URL").unwrap_or_else(|_| "http://127.0.0.1:38147".into());
    let url = reqwest::Url::parse(&base)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !matches!(url.scheme(), "http" | "https")
    {
        return Err(
            "PORCH_URL must be an HTTP(S) origin without credentials, path or query".into(),
        );
    }
    if url.scheme() == "http"
        && !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    {
        return Err("HTTPS required except for loopback development".into());
    }
    if command.starts_with("core-") {
        return tokio::runtime::Runtime::new()?.block_on(core_command(command, &args, &base));
    }
    let cli = Cli {
        client: Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        base: base.trim_end_matches('/').into(),
    };
    if command.starts_with("file-") {
        return files::command(&cli, &args);
    }
    let expected = match command {
        "invite-create" | "message-mutate" | "message-search" | "message-read"
        | "message-embed" => 3,
        "messages" | "message-send" | "message-delta" | "message-pins" | "dm-mutate" => 2,
        "community-invite-create" => 4,
        "community-invite-revoke" => 3,
        "community" | "community-create" | "community-mutate" | "community-invites" => 2,
        "revoke-session" | "invite-revoke" => 2,
        _ => 1,
    };
    if args.len() != expected {
        return Err("incorrect number of arguments".into());
    }
    match command {
        "message-embed" => {
            let conversation: i64 = args[1].parse()?;
            let message: i64 = args[2].parse()?;
            let embed: LinkEmbed = cli
                .request(
                    Method::POST,
                    &format!("/conversations/{conversation}/messages/{message}/embed"),
                    Some(&EmbedRequest {
                        url: env::var("PORCH_LINK")?,
                    }),
                    true,
                )?
                .json()?;
            print_json(&embed)?;
        }
        "communities" => cli.read::<Vec<CommunitySummary>>("/communities")?,
        "community" => {
            let id: i64 = args[1].parse()?;
            cli.read::<CommunitySnapshot>(&format!("/communities/{id}"))?;
        }
        "community-create" => {
            let response: CommunitySummary = cli
                .request(
                    Method::POST,
                    "/communities",
                    Some(&CreateCommunity {
                        name: args[1].clone(),
                    }),
                    true,
                )?
                .json()?;
            print_json(&response)?;
        }
        "community-join" => {
            let response: CommunitySummary = cli
                .request(
                    Method::POST,
                    "/communities/join",
                    Some(&JoinCommunity {
                        code: env::var("PORCH_COMMUNITY_INVITE")?,
                    }),
                    true,
                )?
                .json()?;
            print_json(&response)?;
        }
        "community-mutate" => {
            let id: i64 = args[1].parse()?;
            // Structured protocol command avoids an ever-growing flags dialect.
            let body: CommunityCommand = serde_json::from_str(&env::var("PORCH_COMMAND")?)?;
            let response: CommunityMutation = cli
                .request(
                    Method::POST,
                    &format!("/communities/{id}"),
                    Some(&body),
                    true,
                )?
                .json()?;
            print_json(&response)?;
        }
        "community-invite-create" => {
            let id: i64 = args[1].parse()?;
            let body = CreateInviteRequest {
                max_uses: args[2].parse()?,
                expires_in_seconds: args[3].parse()?,
            };
            let response: CreatedCommunityInvite = cli
                .request(
                    Method::POST,
                    &format!("/communities/{id}/invites"),
                    Some(&body),
                    true,
                )?
                .json()?;
            print_json(&response)?;
        }
        "community-invites" => {
            let id: i64 = args[1].parse()?;
            cli.read::<Vec<CommunityInvite>>(&format!("/communities/{id}/invites"))?;
        }
        "community-invite-revoke" => {
            let id: i64 = args[1].parse()?;
            let invite: i64 = args[2].parse()?;
            cli.delete(&format!("/communities/{id}/invites/{invite}"))?;
        }
        "conversations" => cli.read::<Vec<ConversationHead>>("/conversations")?,
        "dm-create" => {
            let body: CreateDm = serde_json::from_str(&env::var("PORCH_COMMAND")?)?;
            let out: ConversationHead = cli
                .request(Method::POST, "/conversations", Some(&body), true)?
                .json()?;
            print_json(&out)?;
        }
        "dm-mutate" => {
            let id: i64 = args[1].parse()?;
            let body: DmCommand = serde_json::from_str(&env::var("PORCH_COMMAND")?)?;
            cli.request(
                Method::POST,
                &format!("/conversations/{id}"),
                Some(&body),
                true,
            )?;
            println!("ok");
        }
        "messages" | "message-pins" | "message-search" | "message-delta" => {
            let id: i64 = args[1].parse()?;
            let suffix = match command {
                "messages" => "messages",
                "message-pins" => "pins",
                "message-search" => "search",
                _ => "delta",
            };
            let mut url =
                reqwest::Url::parse(&format!("{}/conversations/{id}/{suffix}", cli.base))?;
            if command == "message-search" {
                url.query_pairs_mut().append_pair("q", &args[2]);
            }
            for (env, key) in [
                ("PORCH_BEFORE", "before"),
                ("PORCH_AFTER", "after"),
                ("PORCH_THROUGH", "through"),
                ("PORCH_LIMIT", "limit"),
            ] {
                if let Ok(value) = env::var(env) {
                    let n: i64 = value.parse()?;
                    url.query_pairs_mut().append_pair(key, &n.to_string());
                }
            }
            let path = format!(
                "{}{}",
                url.path(),
                url.query().map(|q| format!("?{q}")).unwrap_or_default()
            );
            cli.read::<MessagePage>(&path)?;
        }
        "message-send" => {
            let id: i64 = args[1].parse()?;
            let body: SendMessage = serde_json::from_str(&env::var("PORCH_MESSAGE")?)?;
            let out: Message = cli
                .request(
                    Method::POST,
                    &format!("/conversations/{id}/messages"),
                    Some(&body),
                    true,
                )?
                .json()?;
            print_json(&out)?;
        }
        "message-mutate" => {
            let id: i64 = args[1].parse()?;
            let message: i64 = args[2].parse()?;
            let body: MessageCommand = serde_json::from_str(&env::var("PORCH_COMMAND")?)?;
            let out: Message = cli
                .request(
                    Method::POST,
                    &format!("/conversations/{id}/messages/{message}"),
                    Some(&body),
                    true,
                )?
                .json()?;
            print_json(&out)?;
        }
        "message-read" => {
            let id: i64 = args[1].parse()?;
            let body = MarkRead {
                message_id: args[2].parse()?,
            };
            cli.request(
                Method::POST,
                &format!("/conversations/{id}/read"),
                Some(&body),
                true,
            )?;
            println!("ok");
        }
        "gateway" => {
            gateway(&cli.base)?;
        }
        "register" => {
            let body = RegisterRequest {
                username: env::var("PORCH_USERNAME")?,
                password: env::var("PORCH_PASSWORD")?,
                invite_code: env::var("PORCH_INVITE")?,
            };
            let result: AuthResponse = cli
                .request(Method::POST, "/auth/register", Some(&body), false)?
                .json()?;
            print_json(&result)?;
        }
        "login" => {
            let body = LoginRequest {
                username: env::var("PORCH_USERNAME")?,
                password: env::var("PORCH_PASSWORD")?,
            };
            let result: AuthResponse = cli
                .request(Method::POST, "/auth/login", Some(&body), false)?
                .json()?;
            print_json(&result)?;
        }
        "me" => cli.read::<User>("/auth/me")?,
        "sessions" => cli.read::<Vec<Session>>("/auth/sessions")?,
        "logout" => {
            cli.request::<()>(Method::POST, "/auth/logout", None, true)?;
            println!("ok");
        }
        "logout-all" => cli.delete("/auth/sessions")?,
        "revoke-session" => {
            let id: i64 = args[1].parse()?;
            cli.delete(&format!("/auth/sessions/{id}"))?;
        }
        "invite-create" => {
            let body = CreateInviteRequest {
                max_uses: args[1].parse()?,
                expires_in_seconds: args[2].parse()?,
            };
            let result: CreatedInvite = cli
                .request(Method::POST, "/invites", Some(&body), true)?
                .json()?;
            print_json(&result)?;
        }
        "invites" => cli.read::<Vec<Invite>>("/invites")?,
        "invite-revoke" => {
            let id: i64 = args[1].parse()?;
            cli.delete(&format!("/invites/{id}"))?;
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}

fn gateway(base: &str) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use porch_protocol::{GatewayClient, GatewayServer, PROTOCOL_VERSION, ResumeCursor};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let url = format!("{}/gateway", base.replacen("http", "ws", 1));
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", env::var("PORCH_TOKEN")?).parse()?,
    );
    let resume = match (env::var("PORCH_STREAM"), env::var("PORCH_SEQUENCE")) {
        (Ok(stream), Ok(sequence)) => Some(ResumeCursor {
            stream,
            sequence: sequence.parse()?,
        }),
        (Err(_), Err(_)) => None,
        _ => return Err("set both PORCH_STREAM and PORCH_SEQUENCE to resume".into()),
    };
    tokio::runtime::Runtime::new()?.block_on(async {
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
        socket.send(Message::Text(serde_json::to_string(&GatewayClient::Identify {version:PROTOCOL_VERSION, resume})?.into())).await?;
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        let mut nonce = 0;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    nonce += 1;
                    socket.send(Message::Text(serde_json::to_string(&GatewayClient::Heartbeat {nonce})?.into())).await?;
                },
                message = socket.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let event: GatewayServer = serde_json::from_str(&text)?;
                        if let GatewayServer::Hello {heartbeat_interval_ms,..} = event {
                            interval = tokio::time::interval(Duration::from_millis(heartbeat_interval_ms.max(1)));
                        }
                        print_json(&event)?;
                        if matches!(event,GatewayServer::Error {..}) { return Err("gateway rejected connection".into()); }
                    },
                    Some(Ok(Message::Ping(payload))) => socket.send(Message::Pong(payload)).await?,
                    Some(Ok(Message::Close(_))) | None => return Err("gateway disconnected".into()),
                    Some(Err(_)) => return Err("gateway transport failed".into()),
                    _ => {},
                }
            }
        }
    })
}

async fn core_command(command: &str, args: &[String], base: &str) -> Result<()> {
    let engine =
        porch_core::Engine::open(base, env::var("PORCH_TOKEN")?, env::var("PORCH_CACHE")?).await?;
    match command {
        "core-sync" => {
            engine.reconcile().await?;
            print_json(
                &serde_json::json!({"heads":engine.cache().heads().await?,"communities":engine.cache().communities().await?}),
            )?;
        }
        "core-cache" => print_json(
            &serde_json::json!({"heads":engine.cache().heads().await?,"communities":engine.cache().communities().await?,"pending":engine.cache().pending().await?,"removal_notice":engine.cache().removal_notice().await?}),
        )?,
        "core-messages" => print_json(
            &engine
                .cache()
                .messages(
                    args.get(1).ok_or("conversation ID required")?.parse()?,
                    None,
                    100,
                )
                .await?,
        )?,
        "core-queue" => {
            let body: SendMessage = serde_json::from_str(&env::var("PORCH_MESSAGE")?)?;
            print_json(
                &engine
                    .queue(
                        args.get(1).ok_or("conversation ID required")?.parse()?,
                        body.content,
                        body.reply_to,
                        body.mentions,
                        body.mention_everyone,
                    )
                    .await?,
            )?;
        }
        "core-flush" => print_json(&serde_json::json!({"sent":engine.flush().await?}))?,
        "core-retry" => {
            engine.retry(args.get(1).ok_or("nonce required")?).await?;
            println!("ok");
        }
        "core-discard" => {
            engine.discard(args.get(1).ok_or("nonce required")?).await?;
            println!("ok");
        }
        "core-logout" => {
            engine.logout().await?;
            print_json(
                &serde_json::json!({"signed_out":true,"discarded_sends":engine.discarded_sends()}),
            )?;
        }
        "core-watch" => {
            let seconds: u64 = env::var("PORCH_WATCH_SECONDS")
                .unwrap_or_else(|_| "30".into())
                .parse()?;
            if !(1..=3600).contains(&seconds) {
                return Err("watch duration must be 1..3600 seconds".into());
            }
            let worker = engine.start_gateway()?;
            let mut state = worker.state();
            let deadline = tokio::time::sleep(Duration::from_secs(seconds));
            tokio::pin!(deadline);
            let mut states = vec![format!("{:?}", *state.borrow())];
            loop {
                tokio::select! {_= &mut deadline=>break,change=state.changed()=>{if change.is_err(){break}states.push(format!("{:?}",*state.borrow()));}}
            }
            worker.stop().await;
            print_json(
                &serde_json::json!({"states":states,"heads":engine.cache().heads().await?,"pending":engine.cache().pending().await?,"removal_notice":engine.cache().removal_notice().await?}),
            )?;
        }
        _ => return Err("unknown core command".into()),
    }
    Ok(())
}
