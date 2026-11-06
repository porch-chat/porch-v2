//! Disposable Postgres, actual loopback HTTP/WebSocket and on-disk SQLite.
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
};
use porch_core::{Api, ConnectionState, Engine, Error};
use porch_protocol::*;
use porch_server::{
    MIGRATOR,
    communities::{self, CommunityService},
    gateway::{EventBus, Gateway},
    identity::{self, IdentityState},
    messaging::MessagingService,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::watch};

async fn wait_state(state: &mut watch::Receiver<ConnectionState>, want: ConnectionState) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if *state.borrow() == want {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_client_engine() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("core_{}", rand::random::<u64>());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await
        .unwrap();
    let search = schema.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
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
    let user = identity::bootstrap(&pool, "engine_admin".into(), "long engine password".into())
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES(20,'engine_peer')")
        .execute(&pool)
        .await
        .unwrap();
    let mut tokens = vec![];
    for id in [user.id, 20] {
        let token = format!("{:064x}", rand::random::<u128>());
        sqlx::query("INSERT INTO sessions(user_id,token_hash) VALUES($1,$2)")
            .bind(id)
            .bind(format!("{:x}", Sha256::digest(token.as_bytes())))
            .execute(&pool)
            .await
            .unwrap();
        tokens.push(token);
    }
    communities::initialize(&pool).await.unwrap();
    let state = IdentityState::new(pool.clone()).await.unwrap();
    let gateway = Gateway::new(state.clone());
    let bus: Arc<dyn EventBus> = Arc::new(gateway.clone());
    state.set_event_bus(&bus);
    let app = CommunityService::new(pool.clone(), state.clone(), bus.clone())
        .routes()
        .merge(MessagingService::new(pool.clone(), state.clone(), bus).routes())
        .merge(identity::routes(state))
        .merge(gateway.routes());
    let lost = Arc::new(AtomicBool::new(false));
    let inject = lost.clone();
    let page_lost = Arc::new(AtomicBool::new(false));
    let page_inject = page_lost.clone();
    let app = app.layer(middleware::from_fn(
        move |req: Request<Body>, next: Next| {
            let inject = inject.clone();
            let page_inject = page_inject.clone();
            async move {
                let messages = req.method() == "POST" && req.uri().path().ends_with("/messages");
                let partial = req.uri().path().ends_with("/delta")
                    && req.uri().query().is_some_and(|q| {
                        q.split('&').any(|s| {
                            s.strip_prefix("after=")
                                .and_then(|s| s.parse::<i64>().ok())
                                .is_some_and(|n| n >= 100)
                        })
                    });
                let response = next.run(req).await;
                if partial && page_inject.swap(false, Ordering::SeqCst) {
                    return Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .body(Body::empty())
                        .unwrap();
                }

                if messages
                    && response.status().is_success()
                    && inject.swap(false, Ordering::SeqCst)
                {
                    return Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .body(Body::empty())
                        .unwrap();
                }
                response
            }
        },
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let server = tokio::spawn({
        let app = app.clone();
        async move { axum::serve(listener, app).await.unwrap() }
    });
    let api = Api::new(&origin, tokens[0].clone()).unwrap();
    let peer = Api::new(&origin, tokens[1].clone()).unwrap();
    let dm: ConversationHead = api
        .post(
            "/conversations",
            &CreateDm {
                participants: vec![20],
                name: None,
            },
        )
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.db");
    let engine = Engine::open(&origin, tokens[0].clone(), &path)
        .await
        .unwrap();
    engine.reconcile().await.unwrap();
    assert!(
        engine
            .cache()
            .heads()
            .await
            .unwrap()
            .iter()
            .any(|h| h.id == dm.id)
    );
    // Commit succeeds but HTTP acknowledgement is lost. Durable nonce survives retry.
    let queued = engine
        .queue(
            dm.id,
            MessageContent::Markdown {
                text: "optimistic hello".into(),
            },
            None,
            vec![],
            false,
        )
        .await
        .unwrap();
    lost.store(true, Ordering::SeqCst);
    assert!(matches!(engine.flush().await, Err(Error::Http(503, _))));
    assert_eq!(engine.cache().pending().await.unwrap().len(), 1);
    assert_eq!(engine.flush().await.unwrap(), 0);
    assert!(engine.cache().pending().await.unwrap().is_empty());
    let messages: MessagePage = api
        .get(&format!("/conversations/{}/messages", dm.id))
        .await
        .unwrap();
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(messages.messages[0].nonce, queued.nonce);
    let mid = messages.messages[0].id;
    let acknowledged: Message = peer
        .post(
            &format!("/conversations/{}/messages", dm.id),
            &SendMessage {
                attachments: vec![],
                nonce: "engine_peer_nonce0001".into(),
                content: MessageContent::Markdown {
                    text: "before ack".into(),
                },
                reply_to: None,
                mentions: vec![],
                mention_everyone: false,
            },
        )
        .await
        .unwrap();
    engine
        .queue(
            dm.id,
            MessageContent::Markdown {
                text: "after peer".into(),
            },
            None,
            vec![],
            false,
        )
        .await
        .unwrap();
    engine.flush().await.unwrap();
    engine.reconcile().await.unwrap();
    assert!(
        engine
            .cache()
            .messages(dm.id, None, 200)
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == acknowledged.id)
    );
    // Multiple delta pages and an interrupted final page cannot advance the cursor early.
    for n in 0..105 {
        let _: Message = api
            .post(
                &format!("/conversations/{}/messages", dm.id),
                &SendMessage {
                    attachments: vec![],
                    nonce: format!("pagination_nonce_{n:06}"),
                    content: MessageContent::Markdown {
                        text: format!("page {n}"),
                    },
                    reply_to: None,
                    mentions: vec![],
                    mention_everyone: false,
                },
            )
            .await
            .unwrap();
    }
    page_lost.store(true, Ordering::SeqCst);
    assert!(matches!(engine.reconcile().await, Err(Error::Http(503, _))));
    assert_eq!(
        engine
            .cache()
            .messages(dm.id, None, 200)
            .await
            .unwrap()
            .len(),
        103
    );
    engine.reconcile().await.unwrap();
    assert_eq!(
        engine
            .cache()
            .messages(dm.id, None, 200)
            .await
            .unwrap()
            .len(),
        108
    );
    // A permanent validation failure remains explicit and does not block later sends.
    let bad = engine
        .queue(
            dm.id,
            MessageContent::Markdown {
                text: "invalid mention".into(),
            },
            None,
            vec![999],
            false,
        )
        .await
        .unwrap();
    engine
        .queue(
            dm.id,
            MessageContent::Markdown {
                text: "later valid message".into(),
            },
            None,
            vec![],
            false,
        )
        .await
        .unwrap();
    assert_eq!(engine.flush().await.unwrap(), 1);
    let pending = engine.cache().pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, porch_core::SendState::Failed);
    engine.discard(&bad.nonce).await.unwrap();
    // Gateway dispatch and edit/delete reconciliation without polling the full history.
    let worker = engine.start_gateway().unwrap();
    assert!(engine.start_gateway().is_err());
    let mut connection = worker.state();
    wait_state(&mut connection, ConnectionState::Online).await;
    let _: Message = api
        .post(
            &format!("/conversations/{}/messages/{mid}", dm.id),
            &MessageCommand::Delete,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if engine
                .cache()
                .messages(dm.id, None, 200)
                .await
                .unwrap()
                .iter()
                .any(|m| m.id == mid && m.deleted)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    worker.stop().await;
    assert_eq!(*connection.borrow(), ConnectionState::Stopped);
    // Offline process restart uses bound profile/cache, but sends reauthenticate first.
    engine
        .queue(
            dm.id,
            MessageContent::Markdown {
                text: "offline draft".into(),
            },
            None,
            vec![],
            false,
        )
        .await
        .unwrap();
    server.abort();
    let _ = server.await;
    drop(engine);
    let offline = Engine::open(&origin, tokens[0].clone(), &path)
        .await
        .unwrap();
    assert_eq!(offline.cache().pending().await.unwrap().len(), 1);
    assert!(
        !offline
            .cache()
            .messages(dm.id, None, 200)
            .await
            .unwrap()
            .is_empty()
    );
    let listener = TcpListener::bind(addr).await.unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    offline.flush().await.unwrap();
    assert!(offline.cache().pending().await.unwrap().is_empty());
    // Community revocation purges cached text and optimistic outbox even on resume.
    let club: CommunitySummary = api
        .post("/communities", &json!({"name":"engine club"}))
        .await
        .unwrap();
    let _:serde_json::Value=api.post(&format!("/communities/{}",club.id),&json!({"op":"create_channel","name":"private","kind":"text","category_id":null,"position":0,"synced":false})).await.unwrap();
    let invite: CreatedCommunityInvite = api
        .post(
            &format!("/communities/{}/invites", club.id),
            &json!({"max_uses":1,"expires_in_seconds":3600}),
        )
        .await
        .unwrap();
    let _: CommunitySummary = peer
        .post("/communities/join", &json!({"code":invite.code}))
        .await
        .unwrap();
    let peer_path = dir.path().join("peer.db");
    let peer_engine = Engine::open(&origin, tokens[1].clone(), &peer_path)
        .await
        .unwrap();
    peer_engine.reconcile().await.unwrap();
    let heads = peer_engine.cache().heads().await.unwrap();
    let club_conversation = heads
        .iter()
        .find(|h| h.community_id == Some(club.id))
        .unwrap()
        .id;
    peer_engine
        .queue(
            club_conversation,
            MessageContent::Markdown {
                text: "revoked queued draft".into(),
            },
            None,
            vec![],
            false,
        )
        .await
        .unwrap();
    let _: serde_json::Value = api
        .post(
            &format!("/communities/{}", club.id),
            &json!({"op":"kick","user_id":20}),
        )
        .await
        .unwrap();
    peer_engine.reconcile().await.unwrap();
    assert!(peer_engine.cache().pending().await.unwrap().is_empty());
    assert!(
        !peer_engine
            .cache()
            .heads()
            .await
            .unwrap()
            .iter()
            .any(|h| h.id == club_conversation)
    );
    // Swapped credentials cannot transmit another account's outbox on reconnect.
    assert!(matches!(
        Engine::open(&origin, tokens[1].clone(), &path).await,
        Err(Error::AccountMismatch)
    ));
    offline
        .queue(
            dm.id,
            MessageContent::Markdown {
                text: "purge on revoke".into(),
            },
            None,
            vec![],
            false,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(offline.reconcile().await.unwrap_err().unauthorized());
    assert!(offline.is_signed_out());
    assert_eq!(offline.discarded_sends(), 1);
    assert!(offline.cache().heads().await.unwrap().is_empty());
    assert!(offline.cache().pending().await.unwrap().is_empty());
    server.abort();
    let _ = server.await;
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
