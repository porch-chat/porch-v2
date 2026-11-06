//! Actual HTTP/extractor + Postgres + local disk, including the disk/SQL crash gap.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use porch_protocol::*;
use porch_server::{
    MIGRATOR,
    embeds::EmbedService,
    files::FileService,
    gateway::EventBus,
    identity::{self, IdentityState},
    messaging::MessagingService,
    storage::{LocalStorage, Storage},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tower::ServiceExt;
struct Bus;
impl EventBus for Bus {
    fn publish(&self, _: &[i64], _: GatewayEvent) {}
}
struct CrashAfterWrite {
    local: LocalStorage,
    fail: AtomicBool,
}
impl Storage for CrashAfterWrite {
    fn put_chunk(&self, key: &str, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.local.put_chunk(key, offset, bytes)?;
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(io::Error::other(
                "simulated process failure after disk commit",
            ));
        }
        Ok(())
    }
    fn chunk_matches(&self, key: &str, offset: u64, bytes: &[u8]) -> io::Result<bool> {
        self.local.chunk_matches(key, offset, bytes)
    }
    fn finalize(&self, key: &str, bytes: u64, sha: &str) -> io::Result<String> {
        self.local.finalize(key, bytes, sha)
    }
    fn open(&self, key: &str) -> io::Result<std::fs::File> {
        self.local.open(key)
    }
    fn remove(&self, key: &str) -> io::Result<()> {
        self.local.remove(key)
    }
}
async fn raw(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    bytes: Vec<u8>,
    range: Option<&str>,
) -> (u16, axum::http::HeaderMap, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    if let Some(range) = range {
        request = request.header("range", range);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(bytes)).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}
async fn call(app: &Router, method: &str, path: &str, token: &str, body: Value) -> (u16, Value) {
    let (status, _, bytes) = raw(
        app,
        method,
        path,
        token,
        body.to_string().into_bytes(),
        None,
    )
    .await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn ok(app: &Router, method: &str, path: &str, token: &str, body: Value) -> Value {
    let (status, value) = call(app, method, path, token, body).await;
    assert!((200..300).contains(&status), "{path} {status} {value}");
    value
}
async fn token(pool: &sqlx::PgPool, id: i64) -> String {
    sqlx::query("INSERT INTO users(id,username) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(id)
        .bind(format!("file_user_{id}"))
        .execute(pool)
        .await
        .unwrap();
    let token = format!("{:064x}", rand::random::<u128>());
    sqlx::query("INSERT INTO sessions(user_id,token_hash) VALUES($1,$2)")
        .bind(id)
        .bind(format!("{:x}", Sha256::digest(token.as_bytes())))
        .execute(pool)
        .await
        .unwrap();
    token
}
fn request(conversation: i64, bytes: &[u8]) -> Value {
    json!({"conversation_id":conversation,"filename":"<script>not-executed.html","bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))})
}
fn send(id: &str, nonce: &str) -> Value {
    json!({"nonce":nonce,"content":{"format":"markdown","text":""},"reply_to":null,"attachments":[id]})
}
#[tokio::test]
#[ignore = "disposable Postgres; CI runs explicitly"]
async fn postgres_files_lifecycle() {
    let url = std::env::var("DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("files_{}", rand::random::<u64>());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await
        .unwrap();
    let search = schema.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |c, _| {
            let q = format!("SET search_path TO {search}");
            Box::pin(async move {
                sqlx::query(&q).execute(c).await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let admin = identity::bootstrap(&pool, "file_admin".into(), "test password for files".into())
        .await
        .unwrap();
    let owner = token(&pool, admin.id).await;
    let peer = token(&pool, 20).await;
    let stranger = token(&pool, 30).await;
    let state = IdentityState::new(pool.clone()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let disk = Arc::new(CrashAfterWrite {
        local: LocalStorage::new(&temp.path().join("private")).unwrap(),
        fail: AtomicBool::new(false),
    });
    let app = MessagingService::new(pool.clone(), state.clone(), Arc::new(Bus))
        .routes()
        .merge(FileService::new(pool.clone(), state.clone(), disk.clone()).routes())
        .merge(EmbedService::new(pool.clone(), state.clone()).routes())
        .merge(identity::routes(state));
    let conversation = ok(
        &app,
        "POST",
        "/conversations",
        &owner,
        json!({"participants":[20],"name":null}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let other = ok(
        &app,
        "POST",
        "/conversations",
        &owner,
        json!({"participants":[30],"name":null}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let messages = format!("/conversations/{conversation}/messages");
    // Embeds must be an explicit link in a live readable message before any
    // outbound connection is considered. Private DNS/IP answers remain forbidden.
    let message=ok(&app,"POST",&messages,&owner,json!({"nonce":"embed-policy-test",
        "content":{"format":"markdown","text":"[private](https://127.0.0.1/) [safe](https://example.com/)"}})).await;
    let mid = message["id"].as_i64().unwrap();
    let embed = format!("{messages}/{mid}/embed");
    assert_eq!(
        call(
            &app,
            "POST",
            &embed,
            &stranger,
            json!({"url":"https://example.com/"})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &embed,
            "",
            json!({"url":"https://example.com/"})
        )
        .await
        .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &embed,
            &owner,
            json!({"url":"https://unlinked.example/"})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &embed,
            &peer,
            json!({"url":"https://127.0.0.1/"})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &embed,
            &owner,
            json!({"url":"http://example.com/"})
        )
        .await
        .0,
        400
    );
    ok(
        &app,
        "POST",
        &format!("{messages}/{mid}"),
        &owner,
        json!({"op":"delete"}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "POST",
            &embed,
            &peer,
            json!({"url":"https://example.com/"})
        )
        .await
        .0,
        404
    );
    // Safe server re-encoding uses the same live scope as original downloads.
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(800, 400)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let up = ok(
        &app,
        "POST",
        "/uploads",
        &owner,
        request(conversation, &png),
    )
    .await;
    let image_id = up["id"].as_str().unwrap();
    assert_eq!(
        raw(
            &app,
            "POST",
            &format!("/uploads/{image_id}/chunks?offset=0"),
            &owner,
            png,
            None
        )
        .await
        .0,
        200
    );
    ok(
        &app,
        "POST",
        &format!("/uploads/{image_id}/complete"),
        &owner,
        Value::Null,
    )
    .await;
    let image_path = format!("/attachments/{image_id}/preview");
    assert_eq!(
        raw(&app, "GET", &image_path, &owner, vec![], None).await.0,
        404
    );
    let image_message = ok(
        &app,
        "POST",
        &messages,
        &owner,
        send(image_id, "files_image_00001"),
    )
    .await;
    let (code, headers, bytes) = raw(&app, "GET", &image_path, &peer, vec![], None).await;
    assert_eq!(code, 200);
    assert_eq!(headers["content-type"], "image/png");
    let preview = image::load_from_memory(&bytes).unwrap();
    assert_eq!((preview.width(), preview.height()), (480, 240));
    assert_eq!(
        raw(&app, "GET", &image_path, &stranger, vec![], None)
            .await
            .0,
        404
    );
    ok(
        &app,
        "POST",
        &format!("{messages}/{}", image_message["id"].as_i64().unwrap()),
        &owner,
        json!({"op":"delete"}),
    )
    .await;
    assert_eq!(
        raw(&app, "GET", &image_path, &peer, vec![], None).await.0,
        404
    );
    let data = b"<html><script>untrusted never inline</script></html>";
    assert_eq!(
        call(&app, "POST", "/uploads", "", request(conversation, data))
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/uploads",
            &stranger,
            request(conversation, data)
        )
        .await
        .0,
        404
    );
    for bad in [0, MAX_UPLOAD_BYTES + 1] {
        let mut body = request(conversation, data);
        body["bytes"] = json!(bad);
        assert_eq!(call(&app, "POST", "/uploads", &owner, body).await.0, 400);
    }
    let upload = ok(
        &app,
        "POST",
        "/uploads",
        &owner,
        request(conversation, data),
    )
    .await;
    let id = upload["id"].as_str().unwrap();
    let path = format!("/uploads/{id}");
    assert_eq!(call(&app, "GET", &path, &peer, Value::Null).await.0, 404);
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/complete"),
            &owner,
            Value::Null
        )
        .await
        .0,
        409
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &messages,
            &owner,
            send(id, "files_pending_0001")
        )
        .await
        .0,
        404
    );
    assert_eq!(
        raw(
            &app,
            "POST",
            &format!("{path}/chunks?offset=1"),
            &owner,
            data.to_vec(),
            None
        )
        .await
        .0,
        400
    );
    disk.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        raw(
            &app,
            "POST",
            &format!("{path}/chunks?offset=0"),
            &owner,
            data.to_vec(),
            None
        )
        .await
        .0,
        500
    );
    assert_eq!(
        ok(&app, "GET", &path, &owner, Value::Null).await["received"],
        0
    );
    assert!(disk.local.chunk_matches(id, 0, data).unwrap());
    for _ in 0..2 {
        assert_eq!(
            raw(
                &app,
                "POST",
                &format!("{path}/chunks?offset=0"),
                &owner,
                data.to_vec(),
                None
            )
            .await
            .0,
            200
        );
    }
    let mut wrong = data.to_vec();
    wrong[0] = b'X';
    assert_eq!(
        raw(
            &app,
            "POST",
            &format!("{path}/chunks?offset=0"),
            &owner,
            wrong,
            None
        )
        .await
        .0,
        409
    );
    for _ in 0..2 {
        assert_eq!(
            ok(
                &app,
                "POST",
                &format!("{path}/complete"),
                &owner,
                Value::Null
            )
            .await["state"],
            "ready"
        );
    }
    assert_eq!(
        raw(
            &app,
            "GET",
            &format!("/attachments/{id}"),
            &owner,
            vec![],
            None
        )
        .await
        .0,
        404
    );
    // Ready does not mean publicly readable or reusable in another conversation.
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/conversations/{other}/messages"),
            &owner,
            send(id, "files_cross_scope1")
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &messages,
            &peer,
            send(id, "files_wrong_owner1")
        )
        .await
        .0,
        404
    );
    let body = send(id, "files_attached_001");
    let (a, b) = tokio::join!(
        call(&app, "POST", &messages, &owner, body.clone()),
        call(&app, "POST", &messages, &owner, body.clone())
    );
    assert_eq!(a.0, 200);
    assert_eq!(b.0, 200);
    assert_eq!(a.1, b.1);
    let message = a.1;
    let mid = message["id"].as_i64().unwrap();
    assert_eq!(message["attachments"][0]["mime"], "text/html");
    assert_eq!(
        call(
            &app,
            "POST",
            &messages,
            &owner,
            send(id, "files_reuse_000001")
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(&app, "DELETE", &path, &owner, Value::Null).await.0,
        409
    );
    let download = format!("/attachments/{id}");
    let (status, headers, actual) = raw(&app, "GET", &download, &peer, vec![], None).await;
    assert_eq!(status, 200);
    assert_eq!(actual, data);
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"porch-download\""
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(
        raw(&app, "GET", &download, &stranger, vec![], None).await.0,
        404
    );
    let (status, headers, actual) =
        raw(&app, "GET", &download, &peer, vec![], Some("bytes=1-9")).await;
    assert_eq!(status, 206);
    assert_eq!(actual, &data[1..10]);
    assert_eq!(
        headers["content-range"],
        format!("bytes 1-9/{}", data.len())
    );
    assert_eq!(
        raw(&app, "GET", &download, &peer, vec![], Some("bytes=999999-"))
            .await
            .0,
        416
    );
    assert_eq!(
        raw(&app, "GET", &download, &peer, vec![], Some("bytes=0-1,3-4"))
            .await
            .0,
        416
    );
    let tombstone = ok(
        &app,
        "POST",
        &format!("{messages}/{mid}"),
        &owner,
        json!({"op":"delete"}),
    )
    .await;
    assert_eq!(tombstone["attachments"], json!([]));
    assert_eq!(
        raw(&app, "GET", &download, &peer, vec![], None).await.0,
        404
    );
    // Hash mismatch never publishes an original.
    let mut bad = request(conversation, b"mismatch");
    bad["sha256"] = json!("0".repeat(64));
    let bad = ok(&app, "POST", "/uploads", &owner, bad).await;
    let bad = bad["id"].as_str().unwrap();
    assert_eq!(
        raw(
            &app,
            "POST",
            &format!("/uploads/{bad}/chunks?offset=0"),
            &owner,
            b"mismatch".to_vec(),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/uploads/{bad}/complete"),
            &owner,
            Value::Null
        )
        .await
        .0,
        400
    );
    assert!(disk.open(bad).is_err());
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/uploads/{bad}"),
            &owner,
            Value::Null
        )
        .await
        .0,
        204
    );
    // Concurrent different chunks serialize and exactly one wins acknowledgement.
    let large = vec![3u8; UPLOAD_CHUNK_BYTES as usize + 7];
    let upload = ok(
        &app,
        "POST",
        "/uploads",
        &peer,
        request(conversation, &large),
    )
    .await;
    let large_id = upload["id"].as_str().unwrap();
    let chunk_path = format!("/uploads/{large_id}/chunks?offset=0");
    let (a, b) = tokio::join!(
        raw(
            &app,
            "POST",
            &chunk_path,
            &peer,
            large[..UPLOAD_CHUNK_BYTES as usize].to_vec(),
            None
        ),
        raw(
            &app,
            "POST",
            &chunk_path,
            &peer,
            vec![4u8; UPLOAD_CHUNK_BYTES as usize],
            None
        )
    );
    assert!((a.0 == 200 && b.0 == 409) || (a.0 == 409 && b.0 == 200));
    assert_eq!(
        ok(
            &app,
            "GET",
            &format!("/uploads/{large_id}"),
            &peer,
            Value::Null
        )
        .await["received"],
        UPLOAD_CHUNK_BYTES
    );
    // Removed participant cannot resume, finalize or read.
    let group = ok(
        &app,
        "POST",
        "/conversations",
        &owner,
        json!({"participants":[20,30],"name":"group"}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let scoped = ok(&app, "POST", "/uploads", &peer, request(group, b"x")).await;
    let scoped = scoped["id"].as_str().unwrap();
    ok(
        &app,
        "POST",
        &format!("/conversations/{group}"),
        &owner,
        json!({"op":"remove","user_id":20}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/uploads/{scoped}"),
            &peer,
            Value::Null
        )
        .await
        .0,
        404
    );
    assert_eq!(
        raw(
            &app,
            "POST",
            &format!("/uploads/{scoped}/chunks?offset=0"),
            &peer,
            b"x".to_vec(),
            None
        )
        .await
        .0,
        404
    );
    // Expiry deletes bytes and frees a reservation at the next creation.
    sqlx::query("UPDATE uploads SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(scoped)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/uploads/{scoped}"),
            &peer,
            Value::Null
        )
        .await
        .0,
        404
    );
    ok(
        &app,
        "POST",
        "/uploads",
        &owner,
        request(conversation, b"cleanup"),
    )
    .await;
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM uploads WHERE id=$1)")
            .bind(scoped)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    // Both reserved byte quotas and pending object count are enforced.
    sqlx::query("UPDATE uploads SET bytes=$1 WHERE id=$2")
        .bind(MAX_UPLOAD_BYTES as i64)
        .bind(large_id)
        .execute(&pool)
        .await
        .unwrap();
    // Owner has at most 16 pending uploads even with tiny objects.
    for _ in 0..20 {
        let (status, _) = call(
            &app,
            "POST",
            "/uploads",
            &owner,
            request(conversation, b"bounded"),
        )
        .await;
        if status == 429 {
            break;
        }
        assert_eq!(status, 200);
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/uploads",
            &owner,
            request(conversation, b"over cap")
        )
        .await
        .0,
        429
    );
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
