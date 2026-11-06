//! Deterministic fault-injected gateway, no database service required.
use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use porch_core::{ConnectionState, Engine};
use porch_protocol::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[derive(Clone, Default)]
struct Fake {
    connections: Arc<AtomicUsize>,
    resumes: Arc<Mutex<Vec<Option<ResumeCursor>>>>,
    syncs: Arc<AtomicUsize>,
}
async fn upgrade(State(fake): State<Fake>, ws: WebSocketUpgrade) -> Response {
    if fake.connections.load(Ordering::SeqCst) >= 2 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| serve(socket, fake))
}
async fn send(socket: &mut WebSocket, value: GatewayServer) {
    socket
        .send(Message::Text(serde_json::to_string(&value).unwrap().into()))
        .await
        .unwrap()
}
async fn serve(mut socket: WebSocket, fake: Fake) {
    let attempt = fake.connections.fetch_add(1, Ordering::SeqCst);
    send(
        &mut socket,
        GatewayServer::Hello {
            version: PROTOCOL_VERSION,
            heartbeat_interval_ms: 100,
            heartbeat_timeout_ms: 500,
        },
    )
    .await;
    let identify = socket.recv().await.unwrap().unwrap();
    let Message::Text(text) = identify else {
        panic!("identify expected")
    };
    let GatewayClient::Identify { resume, .. } = serde_json::from_str(&text).unwrap() else {
        panic!("identify expected")
    };
    fake.resumes.lock().unwrap().push(resume);
    send(
        &mut socket,
        GatewayServer::Ready {
            cursor: ResumeCursor {
                stream: "fake-stream".into(),
                sequence: if attempt == 0 { 0 } else { 3 },
            },
            resumed: attempt != 0,
            presence: vec![],
        },
    )
    .await;
    // Gaps are intentional, typing events are not retained for replay.
    send(
        &mut socket,
        GatewayServer::Dispatch {
            envelope: EventEnvelope {
                version: PROTOCOL_VERSION,
                sequence: if attempt == 0 { 3 } else { 5 },
                event: GatewayEvent::SyncRequired,
            },
        },
    )
    .await;
    let mut beats = 0;
    while let Some(Ok(Message::Text(text))) = socket.recv().await {
        if let GatewayClient::Heartbeat { nonce } = serde_json::from_str(&text).unwrap() {
            send(&mut socket, GatewayServer::HeartbeatAck { nonce }).await;
            beats += 1;
            if beats >= 2 {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
        }
    }
}
#[tokio::test]
async fn reconnect_resume_ephemeral_gaps_and_revocation_stop() {
    let fake = Fake::default();
    let app = Router::new()
        .route(
            "/auth/me",
            get(|| async {
                Json(User {
                    id: 1,
                    username: "mock".into(),
                    admin: false,
                    can_invite: false,
                })
            }),
        )
        .route(
            "/conversations",
            get({
                let f = fake.clone();
                move || {
                    let f = f.clone();
                    async move {
                        f.syncs.fetch_add(1, Ordering::SeqCst);
                        Json(Vec::<ConversationHead>::new())
                    }
                }
            }),
        )
        .route(
            "/communities",
            get(|| async { Json(Vec::<CommunitySummary>::new()) }),
        )
        .route("/gateway", get(upgrade))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(&origin, "test-secret".into(), dir.path().join("cache.db"))
        .await
        .unwrap();
    let worker = engine.start_gateway().unwrap();
    let mut state = worker.state();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if *state.borrow() == ConnectionState::Unauthorized {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    worker.stop().await;
    assert!(engine.is_signed_out());
    {
        let resumes = fake.resumes.lock().unwrap();
        assert_eq!(resumes.len(), 2);
        assert!(resumes[0].is_none());
        assert_eq!(resumes[1].as_ref().unwrap().sequence, 3);
        assert_eq!(resumes[1].as_ref().unwrap().stream, "fake-stream");
    }
    assert!(fake.syncs.load(Ordering::SeqCst) >= 2);
    assert!(engine.cache().heads().await.unwrap().is_empty());
    server.abort();
    let _ = server.await;
}
