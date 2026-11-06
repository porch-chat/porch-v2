//! End-to-end messaging authorization, idempotency races, search and scoped invalidations.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use porch_protocol::*;
use porch_server::{
    MIGRATOR,
    communities::{self, CommunityService},
    gateway::EventBus,
    identity::{self, IdentityState},
    messaging::MessagingService,
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

#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_messaging() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("messaging_{}", rand::random::<u64>());
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
    let member = token(&pool, 20).await;
    let other = token(&pool, 30).await;
    let stranger = token(&pool, 40).await;
    communities::initialize(&pool).await.unwrap();
    let state = IdentityState::new(pool.clone()).await.unwrap();
    let capture = Arc::new(Capture::default());
    let bus: Arc<dyn EventBus> = capture.clone();
    state.set_event_bus(&bus);
    let app = CommunityService::new(pool.clone(), state.clone(), bus.clone())
        .routes()
        .merge(MessagingService::new(pool.clone(), state.clone(), bus).routes())
        .merge(identity::routes(state));
    let club = ok(
        &app,
        "POST",
        "/communities",
        &owner,
        json!({"name":"Message club"}),
    )
    .await;
    let cid = club["id"].as_i64().unwrap();
    let ch=command(&app,cid,&owner,json!({"op":"create_channel","name":"chat","kind":"text","category_id":null,"position":0,"synced":false})).await["id"].as_i64().unwrap();
    let invite = ok(
        &app,
        "POST",
        &format!("/communities/{cid}/invites"),
        &owner,
        json!({"max_uses":2,"expires_in_seconds":3600}),
    )
    .await;
    for t in [&member, &other] {
        ok(
            &app,
            "POST",
            "/communities/join",
            t,
            json!({"code":invite["code"]}),
        )
        .await;
    }
    let heads = ok(&app, "GET", "/conversations", &member, Value::Null).await;
    let conv = heads
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["community_id"] == cid)
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let path = format!("/conversations/{conv}");
    let messages = format!("{path}/messages");
    let official = heads
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["community_id"] != cid)
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let payload = json!({"nonce":"message_nonce_0001","content":{"format":"markdown","text":"**hello** searchable <script>alert(1)</script>"},"reply_to":null,"mentions":[20],"mention_everyone":false});
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/conversations/{official}/messages"),
            &member,
            payload.clone()
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(&app, "GET", &messages, &stranger, Value::Null).await.0,
        404
    );
    capture.0.lock().unwrap().clear();
    // Same nonce races serialize to a single row and single publication.
    let (a, b) = tokio::join!(
        ok(&app, "POST", &messages, &owner, payload.clone()),
        ok(&app, "POST", &messages, &owner, payload.clone())
    );
    assert_eq!(a, b);
    let mid = a["id"].as_i64().unwrap();
    let mp = format!("{messages}/{mid}");
    assert!(!a["rendered_html"].as_str().unwrap().contains("<script"));
    assert!(a["rendered_html"].as_str().unwrap().contains("<strong>"));
    {
        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.len(), 3);
        assert!(!events[0].0.contains(&40));
        assert_eq!(events[0].1, json!({"type":"sync_required"}));
    }
    let mut bad = payload.clone();
    bad["content"]["text"] = json!("changed");
    assert_eq!(call(&app, "POST", &messages, &owner, bad).await.0, 409);
    let mut denied = payload.clone();
    denied["mention_everyone"] = json!(true);
    assert_eq!(call(&app, "POST", &messages, &member, denied).await.0, 403);
    let heads = ok(&app, "GET", "/conversations", &member, Value::Null).await;
    let h = heads
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conv)
        .unwrap();
    assert_eq!(h["unread_count"], 1);
    assert_eq!(h["mention_count"], 1);
    let second=ok(&app,"POST",&messages,&member,json!({"nonce":"message_nonce_0002","content":{"format":"markdown","text":"reply hello"},"reply_to":mid})).await;
    let mid2 = second["id"].as_i64().unwrap();
    assert_eq!(call(&app,"POST",&mp,&member,json!({"op":"edit","content":{"format":"markdown","text":"forged"},"mentions":[],"mention_everyone":false})).await.0,403);
    assert_eq!(
        call(
            &app,
            "POST",
            &mp,
            &member,
            json!({"op":"pin","pinned":true})
        )
        .await
        .0,
        403
    );
    let edit=ok(&app,"POST",&mp,&owner,json!({"op":"edit","content":{"format":"markdown","text":"new text edited"},"mentions":[],"mention_everyone":false})).await;
    assert!(!edit["edited_at"].is_null());
    let (r1, r2) = tokio::join!(
        ok(
            &app,
            "POST",
            &mp,
            &member,
            json!({"op":"react","emoji":"🐈","active":true})
        ),
        ok(
            &app,
            "POST",
            &mp,
            &other,
            json!({"op":"react","emoji":"🐈","active":true})
        )
    );
    assert_ne!(r1["revision"], r2["revision"]);
    let page = ok(&app, "GET", &messages, &owner, Value::Null).await;
    assert_eq!(
        page["messages"][1]["reactions"][0]["users"],
        json!([20, 30])
    );
    ok(
        &app,
        "POST",
        &mp,
        &member,
        json!({"op":"react","emoji":"🐈","active":false}),
    )
    .await;
    ok(&app, "POST", &mp, &owner, json!({"op":"pin","pinned":true})).await;
    let pins = ok(&app, "GET", &format!("{path}/pins"), &member, Value::Null).await;
    assert_eq!(pins["messages"][0]["id"], mid);
    let search = ok(
        &app,
        "GET",
        &format!("{path}/search?q=edited"),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(search["messages"].as_array().unwrap().len(), 1);
    let old = ok(
        &app,
        "GET",
        &format!("{path}/search?q=searchable"),
        &member,
        Value::Null,
    )
    .await;
    assert!(old["messages"].as_array().unwrap().is_empty());
    let p1 = ok(
        &app,
        "GET",
        &format!("{messages}?limit=1"),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(p1["messages"][0]["id"], mid2);
    let p2 = ok(
        &app,
        "GET",
        &format!("{messages}?limit=1&before={}", p1["next"]),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(p2["messages"][0]["id"], mid);
    assert!(p2["next"].is_null());
    let d1 = ok(
        &app,
        "GET",
        &format!("{path}/delta?after=0&limit=1"),
        &member,
        Value::Null,
    )
    .await;
    let high = d1["high_watermark"].as_i64().unwrap();
    let next = d1["next"].as_i64().unwrap();
    assert_eq!(d1["messages"][0]["id"], mid2);
    let d2 = ok(
        &app,
        "GET",
        &format!("{path}/delta?after={next}&through={high}&limit=1"),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(d2["messages"][0]["id"], mid);
    ok(
        &app,
        "POST",
        &format!("{path}/read"),
        &member,
        json!({"message_id":mid2}),
    )
    .await;
    ok(
        &app,
        "POST",
        &format!("{path}/read"),
        &member,
        json!({"message_id":mid}),
    )
    .await;
    let heads = ok(&app, "GET", "/conversations", &member, Value::Null).await;
    let h = heads
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conv)
        .unwrap();
    assert_eq!(h["read_message_id"], mid2);
    assert_eq!(h["unread_count"], 0);
    for query in ["?limit=101", "?limit=0", "?before=-1", "?limit=nope"] {
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("{messages}{query}"),
                &member,
                Value::Null
            )
            .await
            .0,
            400
        );
    }
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("{path}/delta?through=9999"),
            &member,
            Value::Null
        )
        .await
        .0,
        400
    );
    assert_eq!(call(&app,"POST",&messages,&member,json!({"nonce":"opaque_nonce_0001","content":{"format":"ciphertext","algorithm":"fake","payload":"opaque"},"reply_to":null})).await.0,400);
    assert_eq!(call(&app,"POST",&messages,&owner,json!({"nonce":"oversized_nonce_01","content":{"format":"markdown","text":"a".repeat(8001)},"reply_to":null})).await.0,400);
    assert_eq!(call(&app,"POST",&messages,&owner,json!({"nonce":"oversized_nonce_02","content":{"format":"markdown","text":"a".repeat(40000)},"reply_to":null})).await.0,413);
    command(&app,cid,&owner,json!({"op":"set_overwrites","channel_id":ch,"category_id":null,"overwrites":[{"target":{"kind":"member","id":20},"allow":0,"deny":4}]})).await;
    assert_eq!(
        call(&app, "GET", &messages, &member, Value::Null).await.0,
        403
    );
    assert_eq!(call(&app,"POST",&messages,&member,json!({"nonce":"no_history_nonce","content":{"format":"markdown","text":"blocked"},"reply_to":null})).await.0,403);
    // Hidden channels disappear from head lists and every read/mutation endpoint.
    command(&app,cid,&owner,json!({"op":"set_overwrites","channel_id":ch,"category_id":null,"overwrites":[{"target":{"kind":"member","id":20},"allow":0,"deny":1}]})).await;
    for suffix in ["/messages", "/delta", "/search?q=edited", "/pins"] {
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("{path}{suffix}"),
                &member,
                Value::Null
            )
            .await
            .0,
            404
        );
    }
    assert_eq!(
        call(
            &app,
            "POST",
            &mp,
            &member,
            json!({"op":"react","emoji":"x","active":true})
        )
        .await
        .0,
        404
    );
    let heads = ok(&app, "GET", "/conversations", &member, Value::Null).await;
    assert!(!heads.as_array().unwrap().iter().any(|h| h["id"] == conv));
    capture.0.lock().unwrap().clear();
    ok(&app, "POST", &mp, &owner, json!({"op":"delete"})).await;
    {
        let events = capture.0.lock().unwrap();
        assert_eq!(events[0].0, vec![admin.id, 30]);
    }
    let retry = ok(&app, "POST", &messages, &owner, payload).await;
    assert_eq!(retry["deleted"], true);
    assert!(retry["content"].is_null());
    assert!(retry["reactions"].as_array().unwrap().is_empty());
    assert_eq!(retry["pinned"], false);
    assert!(
        ok(
            &app,
            "GET",
            &format!("{path}/search?q=edited"),
            &other,
            Value::Null
        )
        .await["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        ok(&app, "GET", &format!("{path}/pins"), &other, Value::Null).await["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(
            &app,
            "GET",
            &format!("{path}/delta?after={high}"),
            &other,
            Value::Null
        )
        .await["messages"][0]["deleted"],
        true
    );
    // Standalone DMs are unique per unordered pair. They cannot be mutated into groups.
    let (d1, d2) = tokio::join!(
        ok(
            &app,
            "POST",
            "/conversations",
            &member,
            json!({"participants":[30],"name":null})
        ),
        ok(
            &app,
            "POST",
            "/conversations",
            &other,
            json!({"participants":[20],"name":null})
        )
    );
    assert_eq!(d1["id"], d2["id"]);
    let dm = d1["id"].as_i64().unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/conversations/{dm}"),
            &member,
            json!({"op":"add","user_id":40})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/conversations/{dm}/messages"),
            &owner,
            Value::Null
        )
        .await
        .0,
        404
    );
    assert_eq!(call(&app,"POST",&format!("/conversations/{dm}/messages"),&member,json!({"nonce":"wrong_reply_nonce","content":{"format":"markdown","text":"cross reply"},"reply_to":mid2})).await.0,404);
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/conversations/{dm}/read"),
            &member,
            json!({"message_id":mid2})
        )
        .await
        .0,
        404
    );
    let peer_message=ok(&app,"POST",&format!("/conversations/{dm}/messages"),&other,json!({"nonce":"peer_message_0001","content":{"format":"markdown","text":"my message"},"reply_to":null})).await;
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/conversations/{dm}/messages/{}", peer_message["id"]),
            &member,
            json!({"op":"delete"})
        )
        .await
        .0,
        403
    );
    let group = ok(
        &app,
        "POST",
        "/conversations",
        &member,
        json!({"participants":[30,40],"name":"Group"}),
    )
    .await;
    let gid = group["id"].as_i64().unwrap();
    let gp = format!("/conversations/{gid}");
    assert_eq!(
        call(
            &app,
            "POST",
            &gp,
            &stranger,
            json!({"op":"remove","user_id":30})
        )
        .await
        .0,
        403
    );
    let gm=ok(&app,"POST",&format!("{gp}/messages"),&stranger,json!({"nonce":"group_message_0001","content":{"format":"markdown","text":"group history"},"reply_to":null})).await;
    capture.0.lock().unwrap().clear();
    ok(
        &app,
        "POST",
        &gp,
        &member,
        json!({"op":"remove","user_id":40}),
    )
    .await;
    assert!(capture.0.lock().unwrap()[0].0.contains(&40));
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("{gp}/messages"),
            &stranger,
            Value::Null
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{gp}/messages/{}", gm["id"]),
            &stranger,
            json!({"op":"delete"})
        )
        .await
        .0,
        404
    );
    ok(
        &app,
        "POST",
        &gp,
        &member,
        json!({"op":"add","user_id":admin.id}),
    )
    .await;
    ok(
        &app,
        "POST",
        &gp,
        &member,
        json!({"op":"rename","name":"New group"}),
    )
    .await;
    ok(
        &app,
        "POST",
        &gp,
        &member,
        json!({"op":"transfer_owner","user_id":30}),
    )
    .await;
    ok(&app, "POST", &gp, &member, json!({"op":"leave"})).await;
    assert_eq!(
        call(&app, "GET", &format!("{gp}/messages"), &member, Value::Null)
            .await
            .0,
        404
    );
    ok(&app, "POST", &gp, &other, json!({"op":"leave"})).await;
    ok(&app, "POST", &gp, &owner, json!({"op":"leave"})).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE conversation_id=$1")
        .bind(gid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    for (_, event) in capture.0.lock().unwrap().iter() {
        assert_eq!(*event, json!({"type":"sync_required"}));
    }
    // Direct FK enforcement also rejects a cross-conversation reply bypassing HTTP.
    let result=sqlx::query("INSERT INTO messages(conversation_id,author_id,nonce,fingerprint,reply_to,data,revision) VALUES($1,20,'database_nonce_0001',repeat('a',64),$2,'{}',1)").bind(dm).bind(mid2).execute(&pool).await;
    assert!(result.is_err());
    command(&app, cid, &owner, json!({"op":"delete_channel","id":ch})).await;
    assert_eq!(
        call(&app, "GET", &messages, &owner, Value::Null).await.0,
        404
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE conversation_id=$1")
        .bind(conv)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // Representative FTS dataset, not a production capacity claim. Full HTTP route,
    // auth, permission check, query and JSON serialization are included in timings.
    sqlx::query("INSERT INTO messages(conversation_id,author_id,nonce,fingerprint,data,source,revision) SELECT $1,20,lpad(n::text,16,'0'),repeat('a',64),$2,CASE WHEN n%1000=0 THEN 'benchmarkneedle ordinary chat' ELSE 'ordinary chat noise' END,n+100 FROM generate_series(1,20000) n")
        .bind(dm).bind(peer_message.clone()).execute(&pool).await.unwrap();
    sqlx::query("ANALYZE messages")
        .execute(&pool)
        .await
        .unwrap();
    let mut timings = vec![];
    for _ in 0..50 {
        let start = std::time::Instant::now();
        let result = ok(
            &app,
            "GET",
            &format!("/conversations/{dm}/search?q=benchmarkneedle"),
            &member,
            Value::Null,
        )
        .await;
        assert_eq!(result["messages"].as_array().unwrap().len(), 20);
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "20k-message FTS HTTP 50 serial warm queries p50={:.3}ms p95={:.3}ms",
        timings[25], timings[47]
    );
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
