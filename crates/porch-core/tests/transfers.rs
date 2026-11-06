//! Disposable Postgres, actual loopback HTTP/WebSocket and on-disk SQLite.
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
};
use porch_core::{Api, Engine, Error};
use porch_protocol::*;
use porch_server::{
    MIGRATOR,
    communities::{self, CommunityService},
    gateway::{EventBus, Gateway},
    identity::{self, IdentityState},
    messaging::MessagingService,
};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::net::TcpListener;

#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_resumable_transfers() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("transfers_{}", rand::random::<u64>());
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
    let storage_dir = tempfile::tempdir().unwrap();
    let files = porch_server::files::FileService::new(
        pool.clone(),
        state.clone(),
        Arc::new(
            porch_server::storage::LocalStorage::new(&storage_dir.path().join("objects")).unwrap(),
        ),
    );
    let app = CommunityService::new(pool.clone(), state.clone(), bus.clone())
        .routes()
        .merge(MessagingService::new(pool.clone(), state.clone(), bus).routes())
        .merge(files.routes())
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
                let messages = req.method() == "POST" && req.uri().path().ends_with("/chunks");
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
    let cache_path = dir.path().join("cache.db");
    let source = dir.path().join("source.bin");
    let payload = vec![42; UPLOAD_CHUNK_BYTES as usize + 937];
    std::fs::write(&source, &payload).unwrap();
    let engine = Engine::open(&origin, tokens[0].clone(), &cache_path)
        .await
        .unwrap();
    engine.reconcile().await.unwrap();
    let upload = engine.begin_upload(dm.id, &source).await.unwrap();
    assert_eq!(upload.status.received, 0);
    // Real chunk committed by server but HTTP ack fails. No guessed progress.
    lost.store(true, Ordering::SeqCst);
    assert!(matches!(
        engine.advance_upload(&upload.status.id).await,
        Err(Error::Http(503, _))
    ));
    assert_eq!(
        engine.pending_uploads().await.unwrap()[0].status.received,
        0
    );
    // Brand-new engine proves durable metadata plus authoritative resume.
    drop(engine);
    let engine = Engine::open(&origin, tokens[0].clone(), &cache_path)
        .await
        .unwrap();
    let resumed = engine.advance_upload(&upload.status.id).await.unwrap();
    assert_eq!(resumed.status.received, payload.len() as u64);
    let ready = engine.advance_upload(&upload.status.id).await.unwrap();
    assert_eq!(ready.status.state, UploadState::Ready);
    engine
        .queue_attachments(
            dm.id,
            MessageContent::Markdown { text: "".into() },
            None,
            vec![],
            false,
            vec![ready.status.id.clone()],
        )
        .await
        .unwrap();
    assert_eq!(engine.flush().await.unwrap(), 1);
    let page: MessagePage = peer
        .get(&format!("/conversations/{}/messages", dm.id))
        .await
        .unwrap();
    assert_eq!(
        page.messages[0].attachments[0].sha256,
        format!("{:x}", Sha256::digest(&payload))
    );
    assert_eq!(
        engine.cache().messages(dm.id, None, 50).await.unwrap()[0]
            .attachments
            .len(),
        1
    );
    let attached = &page.messages[0];
    let destination = dir.path().join("download.bin");
    engine
        .download_attachment(dm.id, attached.id, &ready.status.id, &destination)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), payload);
    assert!(
        engine
            .download_attachment(dm.id, attached.id, &ready.status.id, &destination)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&destination).unwrap(), payload);
    assert!(
        engine
            .download_attachment(dm.id, attached.id, "../escape", dir.path().join("bad"))
            .await
            .is_err()
    );
    assert!(
        engine
            .attachment_preview(dm.id, attached.id, &ready.status.id)
            .await
            .is_err()
    );
    let pending = engine.begin_upload(dm.id, &source).await.unwrap();
    std::fs::write(&source, b"changed source").unwrap();
    assert!(matches!(
        engine.advance_upload(&pending.status.id).await,
        Err(Error::InvalidData)
    ));
    let status: UploadStatus = api
        .get(&format!("/uploads/{}", pending.status.id))
        .await
        .unwrap();
    assert_eq!(status.received, 0);
    engine.cancel_upload(&pending.status.id).await.unwrap();
    assert_eq!(engine.pending_uploads().await.unwrap().len(), 1);
    // Signed-out cache cascades transfer paths/status, not just messages.
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        engine
            .advance_upload(&ready.status.id)
            .await
            .unwrap_err()
            .unauthorized()
    );
    assert!(engine.is_signed_out());
    let local = sqlx::SqlitePool::connect(&format!("sqlite://{}", cache_path.display()))
        .await
        .unwrap();
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM transfers")
        .fetch_one(&local)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
    local.close().await;
    server.abort();
    let _ = server.await;
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
