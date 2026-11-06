//! Actual adapter commands against disposable Postgres, HTTP, WebSocket and SQLite.
use porch_core::Api;
use porch_flutter::api;
use porch_protocol::*;
use porch_server::{
    MIGRATOR,
    communities::{self, CommunityService},
    gateway::{EventBus, Gateway},
    identity::{self, IdentityState},
    messaging::MessagingService,
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

async fn wait_message(text: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let view: Value = serde_json::from_str(&api::snapshot().await.unwrap()).unwrap();
            if view["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["content"]["text"] == text)
            {
                return view;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("gateway delivered message")
}

#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_flutter_adapter() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("flutter_{}", std::process::id());
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
    let sam = identity::bootstrap(&pool, "flutter_sam".into(), "long flutter password".into())
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
    let login = Api::new(&origin, "login".into()).unwrap();
    let a: AuthResponse = login
        .post(
            "/auth/login",
            &LoginRequest {
                username: "flutter_sam".into(),
                password: "long flutter password".into(),
            },
        )
        .await
        .unwrap();
    let sender = Api::new(&origin, a.token).unwrap();
    let invite: CreatedInvite = sender
        .post(
            "/invites",
            &CreateInviteRequest {
                max_uses: 1,
                expires_in_seconds: 3600,
            },
        )
        .await
        .unwrap();
    let b: AuthResponse = login
        .post(
            "/auth/register",
            &RegisterRequest {
                username: "flutter_maya".into(),
                password: "long flutter password".into(),
                invite_code: invite.code,
            },
        )
        .await
        .unwrap();
    let maya = b.user.clone();
    let remote = Api::new(&origin, b.token).unwrap();
    let dm: ConversationHead = sender
        .post(
            "/conversations",
            &CreateDm {
                participants: vec![maya.id],
                name: None,
            },
        )
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    api::login(
        origin.clone(),
        "flutter_sam".into(),
        "long flutter password".into(),
        dir.path().to_str().unwrap().into(),
    )
    .await
    .unwrap();
    api::select_conversation(dm.id).await.unwrap();
    assert!(api::select_conversation(i64::MAX).await.is_err());
    api::send_message(dm.id, "from Flutter bridge".into())
        .await
        .unwrap();
    let view = wait_message("from Flutter bridge").await;
    assert_eq!(view["user"]["id"], sam.id);
    assert!(!view.to_string().contains("token"));
    let _: Message = remote
        .post(
            &format!("/conversations/{}/messages", dm.id),
            &SendMessage {
                nonce: "flutter_peer_000000000001".into(),
                content: MessageContent::Markdown {
                    text: "from peer gateway".into(),
                },
                mentions: vec![],
                mention_everyone: false,
                attachments: vec![],
                reply_to: None,
            },
        )
        .await
        .unwrap();
    wait_message("from peer gateway").await;
    api::logout().await;
    assert!(
        serde_json::from_str::<Value>(&api::snapshot().await.unwrap()).unwrap()["user"].is_null()
    );
    assert!(
        api::send_message(dm.id, "after logout".into())
            .await
            .is_err()
    );
    api::login(
        origin,
        "flutter_maya".into(),
        "long flutter password".into(),
        dir.path().to_str().unwrap().into(),
    )
    .await
    .unwrap();
    let view: Value = serde_json::from_str(&api::snapshot().await.unwrap()).unwrap();
    assert_eq!(view["user"]["id"], maya.id);
    assert!(view["pending"].as_array().unwrap().is_empty());
    api::logout().await;
    server.abort();
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
