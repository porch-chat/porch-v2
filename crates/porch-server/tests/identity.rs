use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use porch_server::{
    MIGRATOR,
    identity::{IdentityState, bootstrap, routes},
};
use serde_json::{Value, json};
use sqlx::Row;
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if !token.is_empty() {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 16384).await.unwrap();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

async fn app(pool: &sqlx::PgPool) -> Router {
    routes(IdentityState::new(pool.clone()).await.unwrap())
}

const PASSWORD: &str = "integration test password";

#[tokio::test]
#[ignore = "requires disposable Postgres DATABASE_URL; CI runs explicitly"]
async fn postgres_identity() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("identity_{}", rand::random::<u64>());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await
        .unwrap();
    let search = schema.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |conn, _| {
            let query = format!("SET search_path TO {search}");
            Box::pin(async move {
                sqlx::query(&query).execute(conn).await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();

    let admin = bootstrap(&pool, "Admin".into(), PASSWORD.into())
        .await
        .unwrap();
    assert!(admin.admin && admin.can_invite);
    assert!(
        bootstrap(&pool, "other".into(), PASSWORD.into())
            .await
            .is_err()
    );
    let app = app(&pool).await;
    let (status, result) = call(
        &app,
        "POST",
        "/auth/login",
        "",
        json!({"username":"ADMIN","password":PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = result["token"].as_str().unwrap().to_owned();
    let hash: String = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(hash, token);
    assert_eq!(hash.len(), 64);
    let password_hash: String = sqlx::query_scalar("SELECT password_hash FROM identities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(password_hash.starts_with("$argon2id$"));

    for name in ["admin", "missing"] {
        assert_eq!(
            call(
                &app,
                "POST",
                "/auth/login",
                "",
                json!({"username":name,"password":"incorrect password"})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(&app, "GET", "/auth/me", "", json!(null)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/auth/register",
            "",
            json!({"username":"alice","password":PASSWORD,"invite_code":"x".repeat(64)})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, created) = call(
        &app,
        "POST",
        "/invites",
        &token,
        json!({"max_uses":2,"expires_in_seconds":3600}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = created["code"].as_str().unwrap();
    let invite_id = created["invite"]["id"].as_i64().unwrap();
    let listed = call(&app, "GET", "/invites", &token, json!(null)).await.1;
    assert!(!listed.to_string().contains(code));
    assert!(listed[0].get("code_hash").is_none());
    let reg = |name: &str| json!({"username":name,"password":PASSWORD,"invite_code":code});
    let (status, alice) = call(&app, "POST", "/auth/register", "", reg("Alice")).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(alice["user"]["username"], "alice");
    assert_eq!(alice["user"]["admin"], false);
    let alice_token = alice["token"].as_str().unwrap();
    assert_eq!(
        call(&app, "POST", "/auth/register", "", reg("alice"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let uses: i32 = sqlx::query_scalar("SELECT uses FROM invites WHERE id=$1")
        .bind(invite_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 1, "duplicate account rolls back invite use");
    assert_eq!(
        call(
            &app,
            "POST",
            "/invites",
            alice_token,
            json!({"max_uses":1,"expires_in_seconds":3600})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/invites/{invite_id}"),
            alice_token,
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // Concurrent final-use attempts must produce one winner, not two.
    let (one, two) = tokio::join!(
        call(&app, "POST", "/auth/register", "", reg("bob")),
        call(&app, "POST", "/auth/register", "", reg("carol"))
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::CREATED)
            .count(),
        1
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::BAD_REQUEST)
            .count(),
        1
    );
    assert_eq!(
        call(&app, "POST", "/auth/register", "", reg("david"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/invites/{invite_id}"),
            &token,
            json!(null)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );

    // Revoked and expired invites.
    for expired in [false, true] {
        let (_, new) = call(
            &app,
            "POST",
            "/invites",
            &token,
            json!({"max_uses":1,"expires_in_seconds":60}),
        )
        .await;
        let id = new["invite"]["id"].as_i64().unwrap();
        if expired {
            sqlx::query("UPDATE invites SET expires_at=now()-interval '1 second' WHERE id=$1")
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        } else {
            assert_eq!(
                call(
                    &app,
                    "DELETE",
                    &format!("/invites/{id}"),
                    &token,
                    json!(null)
                )
                .await
                .0,
                StatusCode::NO_CONTENT
            );
        }
        assert_eq!(
            call(
                &app,
                "POST",
                "/auth/register",
                "",
                json!({"username":"david","password":PASSWORD,"invite_code":new["code"]})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }

    // Delegation flag is checked live on every operation.
    sqlx::query("UPDATE users SET can_invite=true WHERE id=$1")
        .bind(alice["user"]["id"].as_i64().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/invites",
            alice_token,
            json!({"max_uses":1,"expires_in_seconds":3600})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let own = call(&app, "GET", "/invites", alice_token, json!(null))
        .await
        .1;
    assert_eq!(own.as_array().unwrap().len(), 1);

    let (_, second) = call(
        &app,
        "POST",
        "/auth/login",
        "",
        json!({"username":"alice","password":PASSWORD}),
    )
    .await;
    let second_token = second["token"].as_str().unwrap();
    let sessions = call(&app, "GET", "/auth/sessions", alice_token, json!(null))
        .await
        .1;
    assert_eq!(sessions.as_array().unwrap().len(), 2);
    assert!(!sessions.to_string().contains(alice_token));
    let alice_session = sessions[0]["id"].as_i64().unwrap();
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/auth/sessions/{alice_session}"),
            &token,
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/auth/sessions/{alice_session}"),
            second_token,
            json!(null)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", alice_token, json!(null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "DELETE", "/auth/sessions", second_token, json!(null))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", second_token, json!(null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "POST", "/auth/logout", &token, json!(null))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", &token, json!(null)).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Expired session and persistence across state reconstruction.
    let fresh = app_for(&pool).await;
    let (_, login) = call(
        &fresh,
        "POST",
        "/auth/login",
        "",
        json!({"username":"admin","password":PASSWORD}),
    )
    .await;
    let valid = login["token"].as_str().unwrap();
    let fresh = app_for(&pool).await;
    assert_eq!(
        call(&fresh, "GET", "/auth/me", valid, json!(null)).await.0,
        StatusCode::OK
    );
    sqlx::query("UPDATE sessions SET expires_at=now()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&fresh, "GET", "/auth/me", valid, json!(null)).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Throttling is applied to unsuccessful authentication too.
    let limited = app_for(&pool).await;
    for _ in 0..30 {
        assert_eq!(
            call(
                &limited,
                "POST",
                "/auth/login",
                "",
                json!({"username":"!","password":PASSWORD})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            &limited,
            "POST",
            "/auth/login",
            "",
            json!({"username":"admin","password":PASSWORD})
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let row = sqlx::query("SELECT count(*) AS count FROM identities WHERE provider='password'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("count"), 3);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
}

async fn app_for(pool: &sqlx::PgPool) -> Router {
    app(pool).await
}
