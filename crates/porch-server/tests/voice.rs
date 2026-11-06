//! End-to-end authorization, transactions, invite races and scoped invalidations.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::future::BoxFuture;
use porch_protocol::*;
use porch_server::voice::{VoiceProvider, VoiceService};
use porch_server::{
    MIGRATOR,
    communities::{self, CommunityService},
    gateway::EventBus,
    identity::{self, IdentityState},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
#[derive(Default)]
struct Capture(Mutex<Vec<(Vec<i64>, Value)>>);
impl EventBus for Capture {
    fn publish(&self, users: &[i64], event: GatewayEvent) {
        self.0
            .lock()
            .unwrap()
            .push((users.to_vec(), serde_json::to_value(event).unwrap()));
    }
}
async fn call(app: &Router, method: &str, path: &str, token: &str, body: Value) -> (u16, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    assert_eq!(r.headers()["cache-control"], "no-store");
    let bytes = to_bytes(r.into_body(), 2_097_152).await.unwrap();
    (
        status.as_u16(),
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn ok(app: &Router, method: &str, path: &str, token: &str, body: Value) -> Value {
    let (status, v) = call(app, method, path, token, body).await;
    assert!((200..300).contains(&status), "{path} {status} {v}");
    v
}
async fn command(app: &Router, id: i64, token: &str, cmd: Value) -> Value {
    ok(app, "POST", &format!("/communities/{id}"), token, cmd).await
}
async fn token(pool: &sqlx::PgPool, id: i64) -> String {
    sqlx::query("INSERT INTO users(id,username) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(id)
        .bind(format!("member_{id}"))
        .execute(pool)
        .await
        .unwrap();
    let code = format!("{:064x}", rand::random::<u128>());
    sqlx::query("INSERT INTO sessions(user_id,token_hash) VALUES($1,$2)")
        .bind(id)
        .bind(format!("{:x}", Sha256::digest(code.as_bytes())))
        .execute(pool)
        .await
        .unwrap();
    code
}

#[derive(Default)]
struct Media {
    peers: Mutex<Vec<(String, String)>>,
    removals: Mutex<usize>,
    fail: std::sync::atomic::AtomicBool,
}
impl VoiceProvider for Media {
    fn ticket<'a>(
        &'a self,
        room: &'a str,
        id: &'a str,
        _: bool,
    ) -> BoxFuture<'a, std::result::Result<VoiceTransport, identity::ApiError>> {
        Box::pin(async move {
            self.peers.lock().unwrap().push((room.into(), id.into()));
            Ok(VoiceTransport::Livekit {
                url: "ws://localhost:39780".into(),
                token: "private".into(),
            })
        })
    }
    fn remove<'a>(
        &'a self,
        room: &'a str,
        id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), identity::ApiError>> {
        Box::pin(async move {
            if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(identity::ApiError(ErrorCode::Internal));
            }
            self.peers
                .lock()
                .unwrap()
                .retain(|(r, i)| r != room || i != id);
            *self.removals.lock().unwrap() += 1;
            Ok(())
        })
    }
    fn inventory(
        &self,
    ) -> BoxFuture<'_, std::result::Result<Vec<(String, String)>, identity::ApiError>> {
        Box::pin(async move { Ok(self.peers.lock().unwrap().clone()) })
    }
}
#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_voice_permissions_leases_and_revocation() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("voice_{}", rand::random::<u64>());
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
    let admin = identity::bootstrap(&pool, "admin".into(), "long test password".into())
        .await
        .unwrap();
    let owner = token(&pool, admin.id).await;
    let stranger = token(&pool, 20).await;
    let owner2 = token(&pool, admin.id).await;
    communities::initialize(&pool).await.unwrap();
    let state = IdentityState::new(pool.clone()).await.unwrap();
    let capture = Arc::new(Capture::default());
    let bus: Arc<dyn EventBus> = capture.clone();
    let media = Arc::new(Media::default());
    let voice = VoiceService::new(pool.clone(), state.clone(), bus.clone(), media.clone());
    let app = CommunityService::new(pool.clone(), state.clone(), bus)
        .routes()
        .merge(voice.clone().routes())
        .merge(identity::routes(state));
    let c = ok(
        &app,
        "POST",
        "/communities",
        &owner,
        json!({"name":"voice testing"}),
    )
    .await;
    let cid = c["id"].as_i64().unwrap();
    let v=command(&app,cid,&owner,json!({"op":"create_channel","name":"voice","kind":"voice","category_id":null,"position":0,"synced":false})).await;
    let ch = v["id"].as_i64().unwrap();
    let channel = json!({"community_id":cid,"channel_id":ch});
    assert_eq!(
        call(&app, "POST", "/voice/join", "", json!({"channel":channel}))
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/voice/join",
            &stranger,
            json!({"channel":channel})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/voice/join",
            &owner,
            json!({"channel":{"community_id":cid,"channel_id":999}})
        )
        .await
        .0,
        404
    );
    let ticket = ok(
        &app,
        "POST",
        "/voice/join",
        &owner,
        json!({"channel":channel}),
    )
    .await;
    assert_eq!(ticket["can_speak"], true);
    let lease = ticket["lease"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/voice/join",
            &owner,
            json!({"channel":channel})
        )
        .await
        .0,
        409
    );
    let flags = json!({"muted":false,"deafened":true,"speaking":true});
    assert_eq!(
        call(
            &app,
            "POST",
            "/voice/state",
            &owner2,
            json!({"lease":lease,"flags":flags})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/voice/leave",
            &stranger,
            json!({"lease":lease})
        )
        .await
        .0,
        403
    );
    let f = ok(
        &app,
        "POST",
        "/voice/state",
        &owner,
        json!({"lease":lease,"flags":flags}),
    )
    .await;
    assert_eq!(f, json!({"muted":true,"deafened":true,"speaking":false}));
    let path = format!("/voice/{cid}/{ch}");
    assert_eq!(
        ok(&app, "GET", &path, &owner, Value::Null).await["participants"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        call(&app, "GET", &path, &stranger, Value::Null).await.0,
        403
    );
    media.fail.store(true, std::sync::atomic::Ordering::Relaxed);
    ok(&app, "POST", "/voice/leave", &owner, json!({"lease":lease})).await;
    assert!(voice.sweep().await.is_err());
    assert_eq!(media.peers.lock().unwrap().len(), 1);
    media
        .fail
        .store(false, std::sync::atomic::Ordering::Relaxed);
    voice.sweep().await.unwrap();
    assert!(media.peers.lock().unwrap().is_empty());
    let t = ok(
        &app,
        "POST",
        "/voice/join",
        &owner,
        json!({"channel":channel}),
    )
    .await;
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE token_hash=$1")
        .bind(format!("{:x}", Sha256::digest(owner.as_bytes())))
        .execute(&pool)
        .await
        .unwrap();
    voice.sweep().await.unwrap();
    assert!(media.peers.lock().unwrap().is_empty());
    assert_eq!(
        ok(&app, "GET", &path, &owner2, Value::Null).await["participants"],
        json!([])
    );
    assert_ne!(ticket["lease"], t["lease"]);
    // A previously issued ticket replayed after cleanup is evicted by inventory.
    media
        .peers
        .lock()
        .unwrap()
        .push(("porch-v1-replayed".into(), "stale".into()));
    voice.sweep().await.unwrap();
    assert!(media.peers.lock().unwrap().is_empty());
    assert!(
        capture
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(_, v)| v == &json!({"type":"voice_changed"}))
    );
    drop(app);
    drop(voice);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
}
