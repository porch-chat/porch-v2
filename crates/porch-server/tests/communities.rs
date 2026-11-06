//! End-to-end authorization, transactions, invite races and scoped invalidations.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use porch_protocol::{permissions::*, *};
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

#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_communities() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("communities_{}", rand::random::<u64>());
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
    let outsider = token(&pool, 40).await;
    communities::initialize(&pool).await.unwrap();
    communities::initialize(&pool).await.unwrap();
    let state = IdentityState::new(pool.clone()).await.unwrap();
    let capture = Arc::new(Capture::default());
    let bus: Arc<dyn EventBus> = capture.clone();
    state.set_event_bus(&bus);
    let app = CommunityService::new(pool.clone(), state.clone(), bus)
        .routes()
        .merge(identity::routes(state));
    let official = ok(&app, "GET", "/communities", &member, Value::Null).await;
    assert_eq!(official.as_array().unwrap().len(), 1);
    let oid = official[0]["id"].as_i64().unwrap();
    let official = ok(
        &app,
        "GET",
        &format!("/communities/{oid}"),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(
        official["permissions"].as_u64().unwrap()
            & (SEND_MESSAGES | MANAGE_CHANNELS | CREATE_INVITE),
        0
    );
    assert_eq!(official["members"].as_array().unwrap().len(), 4);
    for cmd in [
        json!({"op":"delete"}),
        json!({"op":"transfer_ownership","user_id":20}),
    ] {
        assert_eq!(
            call(&app, "POST", &format!("/communities/{oid}"), &owner, cmd)
                .await
                .0,
            403
        );
    }
    // Ordinary non-admin users can create communities.
    let own = ok(
        &app,
        "POST",
        "/communities",
        &other,
        json!({"name":"friend group"}),
    )
    .await;
    let own_id = own["id"].as_i64().unwrap();
    command(&app, own_id, &other, json!({"op":"delete"})).await;
    let c = ok(
        &app,
        "POST",
        "/communities",
        &owner,
        json!({"name":"private club"}),
    )
    .await;
    let id = c["id"].as_i64().unwrap();
    let path = format!("/communities/{id}");
    assert_eq!(
        call(&app, "GET", &path, &outsider, Value::Null).await.0,
        404
    );
    assert_eq!(call(&app, "GET", &path, "", Value::Null).await.0, 401);
    assert_eq!(
        call(
            &app,
            "POST",
            "/communities",
            &owner,
            json!({"name":"bad","unknown":true})
        )
        .await
        .0,
        422
    );
    let invite = ok(
        &app,
        "POST",
        &format!("{path}/invites"),
        &owner,
        json!({"max_uses":1,"expires_in_seconds":3600}),
    )
    .await;
    let code = invite["code"].as_str().unwrap();
    let stored: String = sqlx::query_scalar("SELECT code_hash FROM community_invites WHERE id=$1")
        .bind(invite["id"].as_i64().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, code);
    assert_eq!(stored.len(), 64);
    // Distinct simultaneous final-use joins, exactly one succeeds.
    let (a, b) = tokio::join!(
        call(
            &app,
            "POST",
            "/communities/join",
            &member,
            json!({"code":code})
        ),
        call(
            &app,
            "POST",
            "/communities/join",
            &other,
            json!({"code":code})
        )
    );
    assert_eq!([a.0, b.0].iter().filter(|s| **s == 200).count(), 1);
    let (member, member_id, other, other_id) = if a.0 == 200 {
        (member, 20, other, 30)
    } else {
        (other, 30, member, 20)
    };
    let invite2 = ok(
        &app,
        "POST",
        &format!("{path}/invites"),
        &owner,
        json!({"max_uses":10,"expires_in_seconds":3600}),
    )
    .await;
    let code2 = invite2["code"].as_str().unwrap();
    ok(
        &app,
        "POST",
        "/communities/join",
        &other,
        json!({"code":code2}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/communities/join",
            &other,
            json!({"code":code2})
        )
        .await
        .0,
        409
    );
    let uses: i32 = sqlx::query_scalar("SELECT uses FROM community_invites WHERE id=$1")
        .bind(invite2["id"].as_i64().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 1);
    let cat = command(
        &app,
        id,
        &owner,
        json!({"op":"create_category","name":"secret category","position":0}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let ch=command(&app,id,&owner,json!({"op":"create_channel","name":"classified","kind":"text","category_id":cat,"position":0,"synced":true})).await["id"].as_i64().unwrap();
    let voice=command(&app,id,&owner,json!({"op":"create_channel","name":"voice","kind":"voice","category_id":null,"position":0,"synced":false})).await["id"].as_i64().unwrap();
    let role = command(
        &app,
        id,
        &owner,
        json!({"op":"create_role","name":"reader","permissions":VIEW_CHANNEL,"position":1}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    command(&app,id,&owner,json!({"op":"set_overwrites","category_id":cat,"channel_id":null,"overwrites":[{"target":{"kind":"everyone"},"allow":0,"deny":VIEW_CHANNEL},{"target":{"kind":"role","id":role},"allow":VIEW_CHANNEL,"deny":0}]})).await;
    let hidden = ok(&app, "GET", &path, &member, Value::Null).await;
    let serialized = hidden.to_string();
    assert!(!serialized.contains("classified") && !serialized.contains("secret category"));
    assert_eq!(hidden["channels"].as_array().unwrap().len(), 1);
    command(
        &app,
        id,
        &owner,
        json!({"op":"set_member_roles","user_id":member_id,"role_ids":[role]}),
    )
    .await;
    let visible = ok(&app, "GET", &path, &member, Value::Null).await;
    assert!(visible.to_string().contains("classified"));
    capture.0.lock().unwrap().clear();
    command(
        &app,
        id,
        &owner,
        json!({"op":"set_member_roles","user_id":member_id,"role_ids":[]}),
    )
    .await;
    let events = capture.0.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].1, json!({"type":"sync_required"}));
    assert!(events[0].0.contains(&member_id));
    assert!(!events[0].0.contains(&40));
    let hidden = ok(&app, "GET", &path, &member, Value::Null).await;
    assert!(!hidden.to_string().contains("classified"));
    // Revoked role, owner/everyone protections and absent grant permission.
    for cmd in [
        json!({"op":"create_role","name":"evil","permissions":ADMINISTRATOR,"position":1}),
        json!({"op":"set_member_roles","user_id":member_id,"role_ids":[role]}),
        json!({"op":"delete_channel","id":ch}),
        json!({"op":"ban","user_id":admin.id}),
    ] {
        assert_eq!(call(&app, "POST", &path, &member, cmd).await.0, 403);
    }
    let manager=command(&app,id,&owner,json!({"op":"create_role","name":"manager","permissions":MANAGE_ROLES|MANAGE_CHANNELS|KICK_MEMBERS|BAN_MEMBERS,"position":2})).await["id"].as_i64().unwrap();
    command(
        &app,
        id,
        &owner,
        json!({"op":"set_member_roles","user_id":member_id,"role_ids":[manager]}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "POST",
            &path,
            &member,
            json!({"op":"edit_role","id":manager,"name":"takeover","permissions":ALL,"position":2})
        )
        .await
        .0,
        403
    );
    command(
        &app,
        id,
        &member,
        json!({"op":"set_member_roles","user_id":other_id,"role_ids":[role]}),
    )
    .await;
    command(&app, id, &member, json!({"op":"kick","user_id":other_id})).await;
    assert_eq!(call(&app, "GET", &path, &other, Value::Null).await.0, 404);
    ok(
        &app,
        "POST",
        "/communities/join",
        &other,
        json!({"code":code2}),
    )
    .await;
    command(&app, id, &owner, json!({"op":"ban","user_id":other_id})).await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/communities/join",
            &other,
            json!({"code":code2})
        )
        .await
        .0,
        403
    );
    assert!(
        ok(&app, "GET", &path, &member, Value::Null).await["bans"]
            .as_array()
            .unwrap()
            .contains(&json!(other_id))
    );
    command(&app, id, &owner, json!({"op":"unban","user_id":other_id})).await;
    ok(
        &app,
        "POST",
        "/communities/join",
        &other,
        json!({"code":code2}),
    )
    .await;
    command(&app, id, &other, json!({"op":"leave"})).await;
    // Restart service uses persisted aggregate and keeps voluntary official leave.
    command(&app, oid, &other, json!({"op":"leave"})).await;
    communities::initialize(&pool).await.unwrap();
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/communities/{oid}"),
            &other,
            Value::Null
        )
        .await
        .0,
        404
    );
    let reloaded = CommunityService::new(
        pool.clone(),
        IdentityState::new(pool.clone()).await.unwrap(),
        capture.clone(),
    )
    .routes();
    assert_eq!(
        ok(&reloaded, "GET", &path, &owner, Value::Null).await["channels"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Concurrent independent edits don't overwrite each other under row locks.
    let (a, b) = tokio::join!(
        call(
            &app,
            "POST",
            &path,
            &owner,
            json!({"op":"create_role","name":"concurrent_a","permissions":0,"position":1})
        ),
        call(
            &app,
            "POST",
            &path,
            &owner,
            json!({"op":"create_role","name":"concurrent_b","permissions":0,"position":1})
        )
    );
    assert_eq!((a.0, b.0), (200, 200));
    let s = ok(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(s["roles"].as_array().unwrap().len(), 5);
    // Invalid foreign role/category/member references roll back without events.
    let count = capture.0.lock().unwrap().len();
    for cmd in [
        json!({"op":"set_member_roles","user_id":member_id,"role_ids":[999]}),
        json!({"op":"edit_channel","id":voice,"name":"bad","category_id":999,"position":0,"synced":true}),
        json!({"op":"ban","user_id":999999}),
        json!({"op":"delete_category","id":cat}),
    ] {
        assert!(call(&app, "POST", &path, &owner, cmd).await.0 >= 400);
    }
    assert_eq!(capture.0.lock().unwrap().len(), count);
    let all = ok(&app, "GET", &format!("{path}/invites"), &owner, Value::Null).await;
    assert!(!all.to_string().contains("code"));
    assert!(!all.to_string().contains(&stored));
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("{path}/invites"),
            &outsider,
            Value::Null
        )
        .await
        .0,
        404
    );
    ok(
        &app,
        "DELETE",
        &format!("{path}/invites/{}", invite2["id"]),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/communities/join",
            &other,
            json!({"code":code2})
        )
        .await
        .0,
        400
    );
    let exp = ok(
        &app,
        "POST",
        &format!("{path}/invites"),
        &owner,
        json!({"max_uses":1,"expires_in_seconds":60}),
    )
    .await;
    sqlx::query("UPDATE community_invites SET created_at=now()-interval '2 minutes',expires_at=now()-interval '1 minute' WHERE id=$1").bind(exp["id"].as_i64().unwrap()).execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/communities/join",
            &other,
            json!({"code":exp["code"]})
        )
        .await
        .0,
        400
    );
    // Platform invites cannot be used to join a community, and vice versa.
    let p = ok(
        &app,
        "POST",
        "/invites",
        &owner,
        json!({"max_uses":1,"expires_in_seconds":3600}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/communities/join",
            &outsider,
            json!({"code":p["code"]})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/auth/register",
            "",
            json!({"username":"invalid_join","password":"long test password","invite_code":code2})
        )
        .await
        .0,
        400
    );
    // Body caps reject before domain allocations.
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&path)
                .header("content-type", "application/json")
                .body(Body::from(" ".repeat(65537)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    command(&app, id, &owner, json!({"op":"delete_channel","id":ch})).await;
    command(&app, id, &owner, json!({"op":"delete_category","id":cat})).await;
    command(
        &app,
        id,
        &owner,
        json!({"op":"transfer_ownership","user_id":member_id}),
    )
    .await;
    assert_eq!(
        call(&app, "POST", &path, &owner, json!({"op":"delete"}))
            .await
            .0,
        403
    );
    capture.0.lock().unwrap().clear();
    command(&app, id, &member, json!({"op":"delete"})).await;
    assert_eq!(call(&app, "GET", &path, &member, Value::Null).await.0, 404);
    let events = capture.0.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert!(
        events[0].0.contains(&admin.id)
            && events[0].0.contains(&member_id)
            && !events[0].0.contains(&other_id)
    );
    for (_, event) in events {
        assert_eq!(event, json!({"type":"sync_required"}));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM community_invites WHERE community_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
