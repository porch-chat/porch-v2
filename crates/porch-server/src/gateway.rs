//! Single-node ordered dispatcher. Audience selection belongs to feature services,
//! never to untrusted client-supplied subscription IDs.
use crate::identity::{ApiError, IdentityState, authenticate};
use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
    routing::get,
};
use porch_protocol::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;

const REPLAY: usize = 256;
const MAX_SESSIONS: usize = 1024;
const RETENTION: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct Gateway {
    identity: IdentityState,
    inner: Arc<Mutex<Hub>>,
    notify: Arc<Notify>,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
}
struct Hub {
    streams: HashMap<i64, Stream>,
    generation: u64,
}
struct Stream {
    user: i64,
    id: String,
    sequence: u64,
    replay: VecDeque<EventEnvelope>,
    pending: VecDeque<EventEnvelope>,
    active: bool,
    status: PresenceStatus,
    touched: Instant,
}

/// Replace this dispatcher with a shared broker adapter before adding a second
/// node. The adapter must preserve ordering, authorization and resume ownership.
pub trait EventBus: Send + Sync {
    fn publish(&self, audience: &[i64], event: GatewayEvent);
}
impl EventBus for Gateway {
    fn publish(&self, audience: &[i64], event: GatewayEvent) {
        let mut hub = self.inner.lock().unwrap();
        for stream in hub
            .streams
            .values_mut()
            .filter(|s| audience.contains(&s.user))
        {
            stream.dispatch(event.clone());
        }
        self.notify.notify_waiters();
    }
}
impl Stream {
    fn dispatch(&mut self, event: GatewayEvent) {
        self.sequence += 1;
        let envelope = EventEnvelope {
            version: PROTOCOL_VERSION,
            sequence: self.sequence,
            event,
        };
        if !matches!(envelope.event, GatewayEvent::Typing { .. }) {
            self.replay.push_back(envelope.clone());
            if self.replay.len() > REPLAY {
                self.replay.pop_front();
            }
        }
        if self.active {
            self.pending.push_back(envelope);
            // A lagging connection is closed rather than silently losing events.
            if self.pending.len() > REPLAY {
                self.active = false;
                self.pending.clear();
            }
        }
    }
}
impl Hub {
    fn presence(&self) -> Vec<PresenceEntry> {
        let mut statuses = HashMap::new();
        for s in self.streams.values().filter(|s| s.active) {
            let rank = |p| match p {
                PresenceStatus::Dnd => 3,
                PresenceStatus::Online => 2,
                PresenceStatus::Idle => 1,
                PresenceStatus::Offline => 0,
            };
            let p = statuses.entry(s.user).or_insert(PresenceStatus::Offline);
            if rank(s.status) > rank(*p) {
                *p = s.status;
            }
        }
        let mut entries: Vec<_> = statuses
            .into_iter()
            .map(|(user_id, status)| PresenceEntry { user_id, status })
            .collect();
        entries.sort_by_key(|e| e.user_id);
        entries
    }
    fn broadcast(&mut self, event: GatewayEvent) {
        for stream in self.streams.values_mut() {
            stream.dispatch(event.clone());
        }
    }
    fn changed(&mut self, user: i64) {
        let status = self
            .presence()
            .into_iter()
            .find(|e| e.user_id == user)
            .map_or(PresenceStatus::Offline, |e| e.status);
        self.broadcast(GatewayEvent::Presence {
            entry: PresenceEntry {
                user_id: user,
                status,
            },
        });
    }
}
impl Gateway {
    pub fn new(identity: IdentityState) -> Self {
        Self {
            identity,
            inner: Arc::new(Mutex::new(Hub {
                streams: HashMap::new(),
                generation: 0,
            })),
            notify: Arc::new(Notify::new()),
            heartbeat_interval: Duration::from_secs(15),
            heartbeat_timeout: Duration::from_secs(45),
        }
    }
    pub fn routes(self) -> Router {
        Router::new()
            .route("/gateway", get(upgrade))
            .with_state(self)
    }
    fn attach(
        &self,
        user: i64,
        session: i64,
        cursor: Option<ResumeCursor>,
    ) -> Result<GatewayServer, ErrorCode> {
        let mut hub = self.inner.lock().unwrap();
        hub.streams
            .retain(|_, s| s.active || s.touched.elapsed() < RETENTION);
        if hub.streams.get(&session).is_some_and(|s| s.active) {
            return Err(ErrorCode::Conflict);
        }
        if !hub.streams.contains_key(&session) {
            if hub.streams.len() >= MAX_SESSIONS {
                return Err(ErrorCode::RateLimited);
            }
            hub.generation += 1;
            let id = format!("{}-{}", rand::random::<u128>(), hub.generation);
            hub.streams.insert(
                session,
                Stream {
                    user,
                    id,
                    sequence: 0,
                    replay: VecDeque::new(),
                    pending: VecDeque::new(),
                    active: false,
                    status: PresenceStatus::Online,
                    touched: Instant::now(),
                },
            );
        }
        let s = hub.streams.get_mut(&session).unwrap();
        let resumed = cursor.as_ref().is_some_and(|c| {
            c.stream == s.id
                && c.sequence <= s.sequence
                && (s.replay.len() < REPLAY
                    || c.sequence >= s.replay.front().unwrap().sequence.saturating_sub(1))
        });
        s.pending.clear();
        if resumed {
            let c = cursor.unwrap();
            s.pending
                .extend(s.replay.iter().filter(|e| e.sequence > c.sequence).cloned());
        }
        s.active = true;
        s.status = PresenceStatus::Online;
        s.touched = Instant::now();
        let cursor = ResumeCursor {
            stream: s.id.clone(),
            sequence: s.sequence,
        };
        hub.changed(user);
        self.notify.notify_waiters();
        Ok(GatewayServer::Ready {
            cursor,
            resumed,
            presence: hub.presence(),
        })
    }
    fn detach(&self, session: i64) {
        let mut hub = self.inner.lock().unwrap();
        if let Some(s) = hub.streams.get_mut(&session) {
            s.active = false;
            s.pending.clear();
            s.touched = Instant::now();
            let user = s.user;
            hub.changed(user);
        }
        self.notify.notify_waiters();
    }
}

async fn upgrade(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let (user, session) = authenticate(&gateway.identity, &headers).await?;
    Ok(ws
        .max_message_size(4096)
        .max_frame_size(4096)
        .on_upgrade(move |socket| connection(socket, gateway, headers, user.id, session)))
}
async fn send(socket: &mut WebSocket, event: GatewayServer) -> bool {
    let text = serde_json::to_string(&event).unwrap();
    matches!(
        tokio::time::timeout(
            Duration::from_secs(5),
            socket.send(Message::Text(text.into()))
        )
        .await,
        Ok(Ok(()))
    )
}
async fn receive(socket: &mut WebSocket) -> Option<GatewayClient> {
    match socket.recv().await? {
        Ok(Message::Text(text)) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}
async fn connection(
    mut socket: WebSocket,
    gateway: Gateway,
    headers: HeaderMap,
    user: i64,
    session: i64,
) {
    if !send(
        &mut socket,
        GatewayServer::Hello {
            version: PROTOCOL_VERSION,
            heartbeat_interval_ms: gateway.heartbeat_interval.as_millis() as u64,
            heartbeat_timeout_ms: gateway.heartbeat_timeout.as_millis() as u64,
        },
    )
    .await
    {
        return;
    }
    let identify = tokio::time::timeout(Duration::from_secs(5), receive(&mut socket)).await;
    let Ok(Some(GatewayClient::Identify {
        version: PROTOCOL_VERSION,
        resume,
    })) = identify
    else {
        return;
    };
    let ready = match gateway.attach(user, session, resume) {
        Ok(r) => r,
        Err(code) => {
            send(&mut socket, GatewayServer::Error { code }).await;
            return;
        }
    };
    if !send(&mut socket, ready).await {
        gateway.detach(session);
        return;
    }
    let mut heartbeat = Instant::now();
    let mut typing = Instant::now() - Duration::from_secs(10);
    let mut presence = Instant::now() - Duration::from_secs(10);
    let mut tick = tokio::time::interval(gateway.heartbeat_interval);
    loop {
        // Register before draining to avoid lost wakeups.
        let notified = gateway.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let pending = {
            let mut hub = gateway.inner.lock().unwrap();
            let s = hub.streams.get_mut(&session).unwrap();
            if !s.active {
                break;
            }
            s.pending.drain(..).collect::<Vec<_>>()
        };
        let mut failed = false;
        for envelope in pending {
            if !send(&mut socket, GatewayServer::Dispatch { envelope }).await {
                failed = true;
                break;
            }
        }
        if failed {
            break;
        }
        tokio::select! {
            _ = &mut notified => {},
            _ = tick.tick() => {
                if heartbeat.elapsed() >= gateway.heartbeat_timeout || authenticate(&gateway.identity,&headers).await.is_err() { break; }
            },
            message = receive(&mut socket) => {
                match message {
                    Some(GatewayClient::Heartbeat {nonce}) => {
                        heartbeat = Instant::now();
                        if !send(&mut socket,GatewayServer::HeartbeatAck {nonce}).await { break; }
                    },
                    Some(GatewayClient::Presence {status}) if presence.elapsed() >= Duration::from_secs(1) => {
                        presence = Instant::now();
                        let mut hub = gateway.inner.lock().unwrap();
                        hub.streams.get_mut(&session).unwrap().status = status;
                        hub.changed(user);
                        gateway.notify.notify_waiters();
                    },
                    Some(GatewayClient::Typing) if typing.elapsed() >= Duration::from_secs(2) => {
                        typing = Instant::now();
                        let mut hub = gateway.inner.lock().unwrap();
                        hub.broadcast(GatewayEvent::Typing {user_id:user, expires_in_ms:5000});
                        gateway.notify.notify_waiters();
                    },
                    Some(GatewayClient::Typing | GatewayClient::Presence {..}) => {},
                    _ => break,
                }
            }
        }
    }
    gateway.detach(session);
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
}
