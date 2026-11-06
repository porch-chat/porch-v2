//! Server-owned voice control plane. Media-provider credentials never enter logs.
use crate::{
    communities::decode,
    gateway::EventBus,
    identity::{ApiError, IdentityState, authenticate, random_secret},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use futures_util::future::BoxFuture;
use porch_protocol::{permissions::*, *};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

type Result<T> = std::result::Result<T, ApiError>;
fn error(code: ErrorCode) -> ApiError {
    ApiError(code)
}

/// Replace this adapter for another SFU; permissions and leases stay in Porch.
pub trait VoiceProvider: Send + Sync {
    fn ticket<'a>(
        &'a self,
        room: &'a str,
        identity: &'a str,
        can_speak: bool,
    ) -> BoxFuture<'a, Result<VoiceTransport>>;
    fn remove<'a>(&'a self, room: &'a str, identity: &'a str) -> BoxFuture<'a, Result<()>>;
    /// Authoritative SFU inventory, needed to evict stale/replayed credentials.
    fn inventory(&self) -> BoxFuture<'_, Result<Vec<(String, String)>>>;
}
pub struct LiveKitProvider {
    public_url: String,
    key: String,
    secret: String,
    client: livekit_api::services::room::RoomClient,
}
impl LiveKitProvider {
    pub fn new(
        public_url: String,
        internal_url: String,
        key: String,
        secret: String,
    ) -> Result<Self> {
        let url = reqwest::Url::parse(&public_url).map_err(|_| error(ErrorCode::InvalidRequest))?;
        if !(url.scheme() == "wss"
            || (url.scheme() == "ws" && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || key.is_empty()
            || secret.len() < 32
        {
            return Err(error(ErrorCode::InvalidRequest));
        }
        let client =
            livekit_api::services::room::RoomClient::with_api_key(&internal_url, &key, &secret)
                .with_request_timeout(Duration::from_secs(3));
        Ok(Self {
            public_url,
            key,
            secret,
            client,
        })
    }
}
impl VoiceProvider for LiveKitProvider {
    fn ticket<'a>(
        &'a self,
        room: &'a str,
        identity: &'a str,
        can_speak: bool,
    ) -> BoxFuture<'a, Result<VoiceTransport>> {
        Box::pin(async move {
            self.client
                .create_room(
                    room,
                    livekit_api::services::room::CreateRoomOptions {
                        empty_timeout: 60,
                        departure_timeout: 30,
                        max_participants: 200,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| error(ErrorCode::Internal))?;
            let token =
                livekit_api::access_token::AccessToken::with_api_key(&self.key, &self.secret)
                    .with_identity(identity)
                    .with_ttl(Duration::from_secs(60))
                    .with_grants(livekit_api::access_token::VideoGrants {
                        room_join: true,
                        room: room.into(),
                        can_publish: Some(can_speak),
                        can_subscribe: Some(true),
                        can_publish_data: Some(false),
                        can_update_own_metadata: Some(false),
                        can_publish_sources: vec!["microphone".into()],
                        ..Default::default()
                    })
                    .to_jwt()
                    .map_err(|_| error(ErrorCode::Internal))?;
            Ok(VoiceTransport::Livekit {
                url: self.public_url.clone(),
                token,
            })
        })
    }
    fn remove<'a>(&'a self, room: &'a str, identity: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // LiveKit invalidates refreshed tokens for this identity as well.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| error(ErrorCode::Internal))?
                .as_millis() as i64;
            self.client
                .remove_participant_with_options(
                    room,
                    identity,
                    livekit_api::services::room::RemoveParticipantOptions {
                        revoke_token_ts: now + 1,
                    },
                )
                .await
                .map_err(|_| error(ErrorCode::Internal))
        })
    }
    fn inventory(&self) -> BoxFuture<'_, Result<Vec<(String, String)>>> {
        Box::pin(async move {
            let rooms = self
                .client
                .list_rooms(vec![])
                .await
                .map_err(|_| error(ErrorCode::Internal))?;
            let mut peers = Vec::new();
            for room in rooms
                .into_iter()
                .filter(|r| r.name.starts_with("porch-v1-"))
            {
                for p in self
                    .client
                    .list_participants(&room.name)
                    .await
                    .map_err(|_| error(ErrorCode::Internal))?
                {
                    peers.push((room.name.clone(), p.identity));
                }
            }
            Ok(peers)
        })
    }
}
#[derive(Clone)]
struct Lease {
    user: i64,
    session: i64,
    channel: VoiceChannel,
    room: String,
    identity: String,
    can_speak: bool,
    flags: VoiceFlags,
    seen: Instant,
}
struct Leases {
    active: HashMap<String, Lease>,
    window: Instant,
    joins: u32,
}
#[derive(Clone)]
pub struct VoiceService {
    pool: PgPool,
    identity: IdentityState,
    bus: Arc<dyn EventBus>,
    provider: Arc<dyn VoiceProvider>,
    leases: Arc<Mutex<Leases>>,
    epoch: Arc<String>,
}
impl VoiceService {
    pub fn new(
        pool: PgPool,
        identity: IdentityState,
        bus: Arc<dyn EventBus>,
        provider: Arc<dyn VoiceProvider>,
    ) -> Self {
        Self {
            pool,
            identity,
            bus,
            provider,
            leases: Arc::new(Mutex::new(Leases {
                active: HashMap::new(),
                window: Instant::now(),
                joins: 0,
            })),
            epoch: Arc::new(random_secret()),
        }
    }
    pub fn routes(self) -> Router {
        Router::new()
            .route("/voice/join", post(join))
            .route("/voice/state", post(update))
            .route("/voice/leave", post(leave))
            .route("/voice/{community}/{channel}", get(snapshot))
            .layer(DefaultBodyLimit::max(2048))
            .layer(axum::middleware::map_response(
                async |mut r: axum::response::Response| {
                    r.headers_mut()
                        .insert("cache-control", "no-store".parse().unwrap());
                    r
                },
            ))
            .with_state(self)
    }
    /// Run before admitting joins, then once per five seconds. Fail closed on DB errors.
    pub async fn sweep(&self) -> Result<()> {
        let mut leases = self.leases.lock().await;
        let mut dead = Vec::new();
        for (key, l) in &leases.active {
            let valid = sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL AND expires_at>now())")
                .bind(l.session).bind(l.user).fetch_one(&self.pool).await.unwrap_or(false);
            let auth = self.authorize(l.user, l.channel).await;
            if !valid
                || l.seen.elapsed() > Duration::from_secs(30)
                || !matches!(auth,Ok((s,_)) if s==l.can_speak)
            {
                dead.push(key.clone());
            }
        }
        for key in dead {
            if let Some(l) = leases.active.remove(&key) {
                self.notify(l.channel).await;
            }
        }
        // Inventory is authoritative, including peers surviving a Porch restart or
        // replaying a not-yet-expired ticket after leave. Retry failed eviction.
        for (room, identity) in self.provider.inventory().await? {
            if !leases
                .active
                .values()
                .any(|l| l.room == room && l.identity == identity)
            {
                self.provider.remove(&room, &identity).await?;
            }
        }
        Ok(())
    }
    async fn authorize(&self, user: i64, ch: VoiceChannel) -> Result<(bool, Vec<i64>)> {
        let value = sqlx::query_scalar("SELECT data FROM communities WHERE id=$1")
            .bind(ch.community_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| error(ErrorCode::NotFound))?;
        let c = decode(value)?;
        let channel = c
            .channels
            .iter()
            .find(|v| v.id == ch.channel_id && v.kind == ChannelKind::Voice)
            .ok_or_else(|| error(ErrorCode::NotFound))?;
        let bits = c.channel_bits(user, channel);
        if bits & (VIEW_CHANNEL | CONNECT) != (VIEW_CHANNEL | CONNECT) {
            return Err(error(ErrorCode::Forbidden));
        }
        let audience = c
            .members
            .iter()
            .filter(|m| {
                c.channel_bits(m.user_id, channel) & (VIEW_CHANNEL | CONNECT)
                    == (VIEW_CHANNEL | CONNECT)
            })
            .map(|m| m.user_id)
            .collect();
        Ok((bits & SPEAK != 0, audience))
    }
    async fn notify(&self, ch: VoiceChannel) {
        // Invalidations contain no IDs/state and may safely include past members.
        let audience: Vec<i64> =
            sqlx::query_scalar("SELECT user_id FROM community_members WHERE community_id=$1")
                .bind(ch.community_id)
                .fetch_all(&self.pool)
                .await
                .unwrap_or_default();
        self.bus.publish(&audience, GatewayEvent::VoiceChanged);
    }
}
async fn join(
    State(s): State<VoiceService>,
    headers: HeaderMap,
    Json(req): Json<VoiceJoin>,
) -> Result<Json<VoiceTicket>> {
    let (user, session) = authenticate(&s.identity, &headers).await?;
    let mut leases = s.leases.lock().await;
    if leases.window.elapsed() > Duration::from_secs(60) {
        leases.window = Instant::now();
        leases.joins = 0;
    }
    if leases.joins >= 120 || leases.active.len() >= 200 {
        return Err(error(ErrorCode::RateLimited));
    }
    leases.joins += 1;
    // Serialize against community mutations while authorizing/minting the grant.
    let mut tx = s.pool.begin().await?;
    let _: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT data FROM communities WHERE id=$1 FOR SHARE")
            .bind(req.channel.community_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (can_speak, _) = s.authorize(user.id, req.channel).await?;
    if leases.active.values().any(|l| l.user == user.id) {
        return Err(error(ErrorCode::Conflict));
    }
    let lease = random_secret();
    let room = format!(
        "porch-v1-{}-{}-{}",
        s.epoch, req.channel.community_id, req.channel.channel_id
    );
    let identity = format!("u{}-{}", user.id, random_secret());
    let transport = s.provider.ticket(&room, &identity, can_speak).await?;
    tx.commit().await?;
    leases.active.insert(
        lease.clone(),
        Lease {
            user: user.id,
            session,
            channel: req.channel,
            room,
            identity,
            can_speak,
            flags: VoiceFlags {
                muted: true,
                ..Default::default()
            },
            seen: Instant::now(),
        },
    );
    s.notify(req.channel).await;
    Ok(Json(VoiceTicket {
        lease,
        channel: req.channel,
        transport,
        can_speak,
        heartbeat_ms: 10_000,
    }))
}
async fn update(
    State(s): State<VoiceService>,
    headers: HeaderMap,
    Json(req): Json<VoiceUpdate>,
) -> Result<Json<VoiceFlags>> {
    let (user, session) = authenticate(&s.identity, &headers).await?;
    let mut leases = s.leases.lock().await;
    let l = leases
        .active
        .get_mut(&req.lease)
        .filter(|l| l.user == user.id && l.session == session)
        .ok_or_else(|| error(ErrorCode::NotFound))?;
    let (can_speak, _) = s.authorize(user.id, l.channel).await?;
    if can_speak != l.can_speak {
        return Err(error(ErrorCode::Forbidden));
    }
    let flags = req.flags.effective(can_speak);
    let changed = flags != l.flags;
    l.flags = flags;
    l.seen = Instant::now();
    if changed {
        s.notify(l.channel).await;
    }
    Ok(Json(flags))
}
async fn leave(
    State(s): State<VoiceService>,
    headers: HeaderMap,
    Json(req): Json<VoiceLeave>,
) -> Result<Json<bool>> {
    let (user, session) = authenticate(&s.identity, &headers).await?;
    let mut leases = s.leases.lock().await;
    if let Some(l) = leases.active.get(&req.lease) {
        if l.user != user.id || l.session != session {
            return Err(error(ErrorCode::Forbidden));
        }
        let l = leases.active.remove(&req.lease).unwrap();
        // Best-effort immediate removal, inventory sweeper retries even on RPC failure.
        let _ = s.provider.remove(&l.room, &l.identity).await;
        s.notify(l.channel).await;
    }
    Ok(Json(true))
}
async fn snapshot(
    State(s): State<VoiceService>,
    headers: HeaderMap,
    Path((community, channel)): Path<(i64, i64)>,
) -> Result<Json<VoiceSnapshot>> {
    let (user, _) = authenticate(&s.identity, &headers).await?;
    let ch = VoiceChannel {
        community_id: community,
        channel_id: channel,
    };
    let leases = s.leases.lock().await;
    s.authorize(user.id, ch).await?;
    let mut participants = Vec::new();
    for l in leases
        .active
        .values()
        .filter(|l| l.channel == ch && l.seen.elapsed() < Duration::from_secs(30))
    {
        if s.authorize(l.user, ch).await.is_ok() {
            participants.push(VoiceParticipant {
                user_id: l.user,
                flags: l.flags,
            });
        }
    }
    participants.sort_by_key(|p| p.user_id);
    Ok(Json(VoiceSnapshot {
        channel: ch,
        participants,
    }))
}
