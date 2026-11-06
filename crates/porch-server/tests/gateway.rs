//! Real loopback WebSockets and disposable isolated PostgreSQL schemas.
use futures_util::{SinkExt, StreamExt};
use porch_protocol::*;
use porch_server::{
    MIGRATOR,
    gateway::{EventBus, Gateway},
    identity::IdentityState,
};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct Fixture {
    root: sqlx::PgPool,
    pool: sqlx::PgPool,
    schema: String,
    task: tokio::task::JoinHandle<()>,
    url: String,
    gateway: Gateway,
}
impl Fixture {
    async fn new(fast: bool) -> Self {
        let root =
            sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("disposable DATABASE_URL"))
                .await
                .unwrap();
        let schema = format!("gateway_{}", rand::random::<u64>());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&root)
            .await
            .unwrap();
        let search = schema.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .after_connect(move |conn, _| {
                let sql = format!("SET search_path TO {search}");
                Box::pin(async move {
                    sqlx::query(&sql).execute(conn).await?;
                    Ok(())
                })
            })
            .connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let mut gateway = Gateway::new(IdentityState::new(pool.clone()).await.unwrap());
        if fast {
            gateway.heartbeat_interval = Duration::from_millis(40);
            gateway.heartbeat_timeout = Duration::from_millis(800);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/gateway", listener.local_addr().unwrap());
        let app = gateway.clone().routes();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            root,
            pool,
            schema,
            task,
            url,
            gateway,
        }
    }
    async fn token(&self, user: i64) -> String {
        sqlx::query("INSERT INTO users(id,username) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(user)
            .bind(format!("user_{user}"))
            .execute(&self.pool)
            .await
            .unwrap();
        let token = format!("{:064x}", rand::random::<u128>());
        let digest = format!("{:x}", Sha256::digest(token.as_bytes()));
        sqlx::query("INSERT INTO sessions(user_id,token_hash) VALUES($1,$2)")
            .bind(user)
            .bind(digest)
            .execute(&self.pool)
            .await
            .unwrap();
        token
    }
    async fn open(&self, token: &str, resume: Option<ResumeCursor>) -> (Socket, GatewayServer) {
        let mut req = self.url.clone().into_client_request().unwrap();
        req.headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let (mut socket, _) = connect_async(req).await.unwrap();
        assert!(matches!(
            read(&mut socket).await,
            GatewayServer::Hello {
                version: PROTOCOL_VERSION,
                ..
            }
        ));
        write(
            &mut socket,
            GatewayClient::Identify {
                version: PROTOCOL_VERSION,
                resume,
            },
        )
        .await;
        let ready = read(&mut socket).await;
        (socket, ready)
    }
    async fn cleanup(self) {
        self.task.abort();
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.root)
            .await
            .unwrap();
        self.root.close().await;
    }
}
async fn write(s: &mut Socket, event: GatewayClient) {
    s.send(Message::Text(serde_json::to_string(&event).unwrap().into()))
        .await
        .unwrap();
}
async fn read(s: &mut Socket) -> GatewayServer {
    let msg = tokio::time::timeout(Duration::from_secs(3), s.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(msg.to_text().unwrap()).unwrap()
}
async fn dispatch(s: &mut Socket) -> EventEnvelope {
    loop {
        if let GatewayServer::Dispatch { envelope } = read(s).await {
            return envelope;
        }
    }
}
async fn presence(s: &mut Socket, user: i64, status: PresenceStatus) -> EventEnvelope {
    loop {
        let e = dispatch(s).await;
        if matches!(&e.event,GatewayEvent::Presence {entry} if entry.user_id == user && entry.status == status)
        {
            return e;
        }
    }
}
fn cursor(ready: GatewayServer) -> ResumeCursor {
    match ready {
        GatewayServer::Ready { cursor, .. } => cursor,
        _ => panic!("not ready"),
    }
}

#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn gateway_integration() {
    let f = Fixture::new(true).await;
    let token = f.token(1).await;
    let second = f.token(1).await;
    let other = f.token(2).await;
    let mut bad = f.url.clone().into_client_request().unwrap();
    bad.headers_mut()
        .insert("authorization", "Bearer invalid".parse().unwrap());
    assert!(
        matches!(connect_async(bad).await,Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 401)
    );

    let (mut a, ready) = f.open(&token, None).await;
    let mut saved = cursor(ready);
    presence(&mut a, 1, PresenceStatus::Online).await;
    // Same bearer cannot create two simultaneous streams.
    let (_duplicate, rejected) = f.open(&token, None).await;
    assert!(matches!(
        rejected,
        GatewayServer::Error {
            code: ErrorCode::Conflict
        }
    ));
    write(&mut a, GatewayClient::Heartbeat { nonce: 7 }).await;
    assert!(matches!(
        read(&mut a).await,
        GatewayServer::HeartbeatAck { nonce: 7 }
    ));

    let (mut b, _) = f.open(&second, None).await;
    presence(&mut a, 1, PresenceStatus::Online).await;
    write(
        &mut b,
        GatewayClient::Presence {
            status: PresenceStatus::Dnd,
        },
    )
    .await;
    let e = presence(&mut a, 1, PresenceStatus::Dnd).await;
    saved.sequence = e.sequence;
    // Disconnect this session, then create events while it is offline.
    a.close(None).await.unwrap();
    presence(&mut b, 1, PresenceStatus::Dnd).await;
    let (mut c, _) = f.open(&other, None).await;
    presence(&mut b, 2, PresenceStatus::Online).await;
    let (mut a, ready) = f.open(&token, Some(saved.clone())).await;
    assert!(matches!(ready, GatewayServer::Ready { resumed: true, .. }));
    let e = dispatch(&mut a).await;
    assert!(e.sequence > saved.sequence);
    saved.sequence = e.sequence;
    presence(&mut a, 2, PresenceStatus::Online).await;
    // Ephemeral typing broadcast and burst suppression.
    presence(&mut c, 1, PresenceStatus::Dnd).await;
    write(&mut b, GatewayClient::Typing).await;
    let e = dispatch(&mut c).await;
    assert!(matches!(e.event, GatewayEvent::Typing { user_id: 1, .. }));
    write(&mut b, GatewayClient::Typing).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), c.next())
            .await
            .is_err()
    );

    // Keep heartbeat-alive client while others time out; aggregation becomes offline.
    write(&mut c, GatewayClient::Heartbeat { nonce: 9 }).await;
    loop {
        if matches!(read(&mut c).await, GatewayServer::HeartbeatAck { nonce: 9 }) {
            break;
        }
    }
    // A/b must close without heartbeats. A may still have queued dispatch frames.
    loop {
        match tokio::time::timeout(Duration::from_secs(2), a.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Close(_))) | None => break,
            Some(Ok(_)) => {}
            x => panic!("{x:?}"),
        }
    }
    // Stale buffer cursor requires a fresh snapshot, not partial replay.
    for _ in 0..300 {
        f.gateway.publish(
            &[1],
            GatewayEvent::Presence {
                entry: PresenceEntry {
                    user_id: 2,
                    status: PresenceStatus::Idle,
                },
            },
        );
    }
    let (mut resumed, ready) = f.open(&token, Some(saved)).await;
    assert!(matches!(ready, GatewayServer::Ready { resumed: false, .. }));
    // Revocation closes established sockets at the next auth/heartbeat check.
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=1")
        .execute(&f.pool)
        .await
        .unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(2), resumed.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Close(_))) | None => break,
            Some(Ok(_)) => {}
            x => panic!("{x:?}"),
        }
    }
    drop((a, b, c, resumed));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual 200-client load benchmark with disposable Postgres"]
async fn gateway_load_200() {
    let f = Fixture::new(false).await;
    let mut tokens = Vec::new();
    for user in 1..=200 {
        tokens.push(f.token(user).await);
    }
    let barrier = Arc::new(tokio::sync::Barrier::new(201));
    let broadcast_start = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let mut clients = Vec::new();
    for token in tokens {
        let mut req = f.url.clone().into_client_request().unwrap();
        req.headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let barrier = barrier.clone();
        let broadcast_start = broadcast_start.clone();
        clients.push(tokio::spawn(async move {
            let (mut s, _) = connect_async(req).await.unwrap();
            read(&mut s).await;
            write(
                &mut s,
                GatewayClient::Identify {
                    version: PROTOCOL_VERSION,
                    resume: None,
                },
            )
            .await;
            assert!(matches!(read(&mut s).await, GatewayServer::Ready { .. }));
            barrier.wait().await;
            loop {
                let e = dispatch(&mut s).await;
                if matches!(
                    e.event,
                    GatewayEvent::Presence {
                        entry: PresenceEntry { user_id: 999, .. }
                    }
                ) {
                    break;
                }
            }
            let fanout = broadcast_start
                .lock()
                .unwrap()
                .unwrap()
                .elapsed()
                .as_micros() as u64;
            let mut samples = Vec::new();
            for nonce in 0..20 {
                let start = Instant::now();
                write(&mut s, GatewayClient::Heartbeat { nonce }).await;
                loop {
                    if let GatewayServer::HeartbeatAck { nonce: actual } = read(&mut s).await {
                        assert_eq!(actual, nonce);
                        break;
                    }
                }
                samples.push(start.elapsed().as_micros() as u64);
            }
            barrier.wait().await;
            s.close(None).await.unwrap();
            (samples, fanout)
        }));
    }
    barrier.wait().await;
    *broadcast_start.lock().unwrap() = Some(Instant::now());
    f.gateway.publish(
        &(1..=200).collect::<Vec<_>>(),
        GatewayEvent::Presence {
            entry: PresenceEntry {
                user_id: 999,
                status: PresenceStatus::Online,
            },
        },
    );
    barrier.wait().await;
    let mut samples = Vec::new();
    let mut fanout = Vec::new();
    for client in clients {
        let (rtt, latency) = client.await.unwrap();
        samples.extend(rtt);
        fanout.push(latency);
    }
    fanout.sort_unstable();
    println!(
        "200-recipient bus dispatch latency (microseconds): p50={} p95={} p99={} max={}",
        fanout[100], fanout[190], fanout[198], fanout[199]
    );
    samples.sort_unstable();
    println!(
        "200 concurrent real authenticated WS clients; {} heartbeat RTT samples (microseconds): p50={} p95={} p99={} max={}",
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() * 99 / 100],
        samples.last().unwrap()
    );
    f.cleanup().await;
}
