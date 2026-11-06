//! Authenticated, conversation-scoped file lifecycle. No public object URLs.
use crate::{
    identity::{ApiError, IdentityState, authenticate},
    messaging::context,
    storage::{Storage, byte_range, seek_range, valid_key},
};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use porch_protocol::{permissions::*, *};
use rand::RngCore;
use serde::Deserialize;
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::{
    io::AsyncReadExt,
    sync::{OwnedSemaphorePermit, Semaphore},
};
type Result<T> = std::result::Result<T, ApiError>;
fn error(code: ErrorCode) -> ApiError {
    ApiError(code)
}
fn invalid() -> ApiError {
    error(ErrorCode::InvalidRequest)
}
fn missing() -> ApiError {
    error(ErrorCode::NotFound)
}
fn io_error(_: std::io::Error) -> ApiError {
    error(ErrorCode::Internal)
}

#[derive(Clone)]
pub struct FileService {
    pool: PgPool,
    identity: IdentityState,
    storage: Arc<dyn Storage>,
    work: Arc<Semaphore>,
    video: Option<crate::previews::VideoPreviewer>,
}
impl FileService {
    pub fn new(pool: PgPool, identity: IdentityState, storage: Arc<dyn Storage>) -> Self {
        Self {
            pool,
            identity,
            storage,
            work: Arc::new(Semaphore::new(8)),
            video: None,
        }
    }
    pub fn with_video_previews(mut self, video: crate::previews::VideoPreviewer) -> Self {
        self.video = Some(video);
        self
    }
    pub fn routes(self) -> Router {
        Router::new()
            .route("/uploads", post(create))
            .route("/uploads/{id}", get(status).delete(abort))
            .route("/uploads/{id}/chunks", post(chunk))
            .route("/uploads/{id}/complete", post(complete))
            .route("/attachments/{id}", get(download))
            .route("/attachments/{id}/preview", get(preview))
            .layer(axum::extract::DefaultBodyLimit::max(4096))
            .layer(axum::middleware::map_response(async |mut r: Response| {
                r.headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                r.headers_mut()
                    .insert("x-content-type-options", "nosniff".parse().unwrap());
                r
            }))
            .with_state(self)
    }
    fn permit(&self) -> Result<OwnedSemaphorePermit> {
        self.work
            .clone()
            .try_acquire_owned()
            .map_err(|_| error(ErrorCode::RateLimited))
    }
    async fn disk<T: Send + 'static>(
        &self,
        f: impl FnOnce(Arc<dyn Storage>) -> std::io::Result<T> + Send + 'static,
    ) -> Result<T> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || f(storage))
            .await
            .map_err(|_| error(ErrorCode::Internal))?
            .map_err(io_error)
    }
}
fn valid_filename(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 180
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn decode(row: &PgRow) -> Result<UploadStatus> {
    Ok(UploadStatus {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        filename: row.get("filename"),
        bytes: row.get::<i64, _>("bytes") as u64,
        received: row.get::<i64, _>("received") as u64,
        chunk_bytes: UPLOAD_CHUNK_BYTES,
        sha256: row.get("sha256"),
        expires_at: row.get("expiry"),
        state: match row.get::<&str, _>("state") {
            "receiving" => UploadState::Receiving,
            "ready" => UploadState::Ready,
            "attached" => UploadState::Attached,
            _ => return Err(error(ErrorCode::Internal)),
        },
    })
}
async fn owned(conn: &mut PgConnection, id: &str, user: i64, write: bool) -> Result<UploadStatus> {
    if !valid_key(id) {
        return Err(missing());
    }
    // Immutable routing hint, then shared conversation lock, then upload row.
    let conversation: i64 =
        sqlx::query_scalar("SELECT conversation_id FROM uploads WHERE id=$1 AND owner_id=$2")
            .bind(id)
            .bind(user)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(missing)?;
    let ctx = context(conn, conversation, user, true).await?;
    ctx.require(READ_HISTORY | if write { SEND_MESSAGES } else { 0 })?;
    let row=sqlx::query("SELECT *,expires_at::text AS expiry,expires_at>now() AS alive FROM uploads WHERE id=$1 AND owner_id=$2 FOR UPDATE")
        .bind(id).bind(user).fetch_optional(conn).await?.ok_or_else(missing)?;
    if !row.get::<bool, _>("alive") && row.get::<Option<i64>, _>("message_id").is_none() {
        return Err(missing());
    }
    decode(&row)
}
async fn create(
    State(s): State<FileService>,
    h: HeaderMap,
    Json(body): Json<CreateUpload>,
) -> Result<Json<UploadStatus>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let _permit = s.permit()?;
    if !valid_filename(&body.filename)
        || !valid_hash(&body.sha256)
        || body.bytes == 0
        || body.bytes > MAX_UPLOAD_BYTES
    {
        return Err(invalid());
    }
    let mut tx = s.pool.begin().await?;
    context(&mut tx, body.conversation_id, u.id, true)
        .await?
        .require(READ_HISTORY | SEND_MESSAGES)?;
    // Serial reservation accounting across all instances sharing this database.
    sqlx::query("SELECT pg_advisory_xact_lock(7077001)")
        .execute(&mut *tx)
        .await?;
    let expired:Vec<String>=sqlx::query_scalar("SELECT id FROM uploads WHERE message_id IS NULL AND expires_at<=now() ORDER BY expires_at LIMIT 32 FOR UPDATE SKIP LOCKED")
        .fetch_all(&mut *tx).await?;
    for id in expired {
        let key = id.clone();
        s.disk(move |store| store.remove(&key)).await?;
        sqlx::query("DELETE FROM uploads WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let row=sqlx::query("SELECT coalesce(sum(bytes),0)::bigint AS total,coalesce(sum(bytes) FILTER(WHERE owner_id=$1),0)::bigint AS own,count(*) FILTER(WHERE owner_id=$1 AND message_id IS NULL) AS pending FROM uploads")
        .bind(u.id).fetch_one(&mut *tx).await?;
    if row.get::<i64, _>("total") + body.bytes as i64 > 100 * 1024 * 1024 * 1024
        || row.get::<i64, _>("own") + body.bytes as i64 > 10 * 1024 * 1024 * 1024
        || row.get::<i64, _>("pending") >= 16
    {
        return Err(error(ErrorCode::RateLimited));
    }
    let mut random = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let id = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let row=sqlx::query("INSERT INTO uploads(id,owner_id,conversation_id,filename,bytes,sha256) VALUES($1,$2,$3,$4,$5,$6) RETURNING *,expires_at::text AS expiry")
        .bind(id).bind(u.id).bind(body.conversation_id).bind(body.filename).bind(body.bytes as i64).bind(body.sha256)
        .fetch_one(&mut *tx).await?;
    let out = decode(&row)?;
    tx.commit().await?;
    Ok(Json(out))
}
async fn status(
    State(s): State<FileService>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<UploadStatus>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let _permit = s.permit()?;
    let mut tx = s.pool.begin().await?;
    let out = owned(&mut tx, &id, u.id, false).await?;
    tx.commit().await?;
    Ok(Json(out))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkQuery {
    offset: u64,
}
async fn chunk(
    State(s): State<FileService>,
    h: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<ChunkQuery>,
    body: Body,
) -> Result<Json<UploadStatus>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let _permit = s.permit()?;
    let bytes = tokio::time::timeout(
        Duration::from_secs(15),
        to_bytes(body, UPLOAD_CHUNK_BYTES as usize),
    )
    .await
    .map_err(|_| invalid())?
    .map_err(|_| invalid())?;
    if bytes.is_empty() || !q.offset.is_multiple_of(UPLOAD_CHUNK_BYTES) {
        return Err(invalid());
    }
    let mut tx = s.pool.begin().await?;
    let mut out = owned(&mut tx, &id, u.id, true).await?;
    if out.state != UploadState::Receiving {
        return Err(error(ErrorCode::Conflict));
    }
    let expected = (out.bytes.saturating_sub(q.offset)).min(UPLOAD_CHUNK_BYTES);
    if expected == 0 || bytes.len() as u64 != expected {
        return Err(invalid());
    }
    if q.offset < out.received {
        let key = id.clone();
        let matches = s
            .disk(move |store| store.chunk_matches(&key, q.offset, &bytes))
            .await?;
        if !matches {
            return Err(error(ErrorCode::Conflict));
        }
    } else if q.offset == out.received {
        let key = id.clone();
        s.disk(move |store| store.put_chunk(&key, q.offset, &bytes))
            .await?;
        out.received += expected;
        sqlx::query("UPDATE uploads SET received=$2 WHERE id=$1")
            .bind(&id)
            .bind(out.received as i64)
            .execute(&mut *tx)
            .await?;
    } else {
        return Err(error(ErrorCode::Conflict));
    }
    tx.commit().await?;
    Ok(Json(out))
}
async fn complete(
    State(s): State<FileService>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<UploadStatus>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let _permit = s.permit()?;
    let mut tx = s.pool.begin().await?;
    let mut out = owned(&mut tx, &id, u.id, true).await?;
    if out.state == UploadState::Receiving {
        if out.received != out.bytes {
            return Err(error(ErrorCode::Conflict));
        }
        let key = id.clone();
        let bytes = out.bytes;
        let hash = out.sha256.clone();
        // Invalid size/hash is a request error, not a leaked filesystem diagnostic.
        let storage = s.storage.clone();
        let mime = tokio::task::spawn_blocking(move || storage.finalize(&key, bytes, &hash))
            .await
            .map_err(|_| error(ErrorCode::Internal))?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::InvalidInput {
                    invalid()
                } else {
                    io_error(e)
                }
            })?;
        sqlx::query("UPDATE uploads SET state='ready',mime=$2 WHERE id=$1")
            .bind(&id)
            .bind(mime)
            .execute(&mut *tx)
            .await?;
        out.state = UploadState::Ready;
    }
    tx.commit().await?;
    Ok(Json(out))
}
async fn abort(
    State(s): State<FileService>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let _permit = s.permit()?;
    let mut tx = s.pool.begin().await?;
    let out = owned(&mut tx, &id, u.id, true).await?;
    if out.state == UploadState::Attached {
        return Err(error(ErrorCode::Conflict));
    }
    let key = id.clone();
    s.disk(move |store| store.remove(&key)).await?;
    sqlx::query("DELETE FROM uploads WHERE id=$1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Called only while the conversation's mutation lock is held.
pub(crate) async fn bind_attachments(
    conn: &mut PgConnection,
    ids: &[String],
    user: i64,
    conversation: i64,
    message: i64,
) -> Result<Vec<Attachment>> {
    if ids.len() > 10 || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(invalid());
    }
    let mut out = vec![];
    for id in ids {
        if !valid_key(id) {
            return Err(invalid());
        }
        let row=sqlx::query("SELECT * FROM uploads WHERE id=$1 AND owner_id=$2 AND conversation_id=$3 AND state='ready' AND expires_at>now() FOR UPDATE")
            .bind(id).bind(user).bind(conversation).fetch_optional(&mut *conn).await?.ok_or_else(missing)?;
        out.push(Attachment {
            id: id.clone(),
            filename: row.get("filename"),
            bytes: row.get::<i64, _>("bytes") as u64,
            sha256: row.get("sha256"),
            mime: row.get("mime"),
        });
        sqlx::query("UPDATE uploads SET state='attached',message_id=$2 WHERE id=$1")
            .bind(id)
            .bind(message)
            .execute(&mut *conn)
            .await?;
    }
    Ok(out)
}
async fn preview(
    State(s): State<FileService>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let _permit = s.permit()?;
    if !valid_key(&id) {
        return Err(missing());
    }
    let mut tx = s.pool.begin().await?;
    let row = sqlx::query(
        "SELECT conversation_id,message_id,bytes,mime FROM uploads WHERE id=$1 AND state='attached'",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(missing)?;
    let conversation: i64 = row.get("conversation_id");
    // Hold the same conversation ordering as send/delete while opening bytes.
    context(&mut tx, conversation, u.id, true)
        .await?
        .require(READ_HISTORY)?;
    let alive: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=$1 AND id=$2 AND NOT deleted)",
    )
    .bind(conversation)
    .bind(row.get::<i64, _>("message_id"))
    .fetch_one(&mut *tx)
    .await?;
    if !alive {
        return Err(missing());
    }
    let bytes = row.get::<i64, _>("bytes") as u64;
    let mime: String = row.get("mime");
    let png = if matches!(mime.as_str(), "video/mp4" | "video/webm") {
        if bytes > crate::previews::MAX_VIDEO_SOURCE {
            return Err(invalid());
        }
        let decoder = s.video.as_ref().ok_or_else(invalid)?;
        let file = s.disk(move |store| store.open(&id)).await?;
        decoder.poster(file).await.map_err(|_| invalid())?
    } else {
        if bytes > crate::previews::MAX_PREVIEW_SOURCE {
            return Err(invalid());
        }
        s.disk(move |store| crate::previews::thumbnail(store.open(&id)?))
            .await?
    };
    tx.commit().await?;
    Ok((
        [
            ("content-type", "image/png"),
            ("content-disposition", "inline; filename=\"preview.png\""),
        ],
        png,
    )
        .into_response())
}
async fn download(
    State(s): State<FileService>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let permit = s.permit()?;
    if !valid_key(&id) {
        return Err(missing());
    }
    let mut tx = s.pool.begin().await?;
    let row = sqlx::query(
        "SELECT conversation_id,message_id,bytes,mime FROM uploads WHERE id=$1 AND state='attached'",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(missing)?;
    let conversation: i64 = row.get("conversation_id");
    // Hold the same conversation ordering as send/delete while opening bytes.
    context(&mut tx, conversation, u.id, true)
        .await?
        .require(READ_HISTORY)?;
    let alive: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=$1 AND id=$2 AND NOT deleted)",
    )
    .bind(conversation)
    .bind(row.get::<i64, _>("message_id"))
    .fetch_one(&mut *tx)
    .await?;
    if !alive {
        return Err(missing());
    }
    let bytes = row.get::<i64, _>("bytes") as u64;
    let range = h
        .get("range")
        .map(|h| h.to_str().map_err(|_| invalid()))
        .transpose()?;
    let (start, end) = match byte_range(range, bytes) {
        Ok(r) => r,
        Err(_) => {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [("content-range", format!("bytes */{bytes}"))],
            )
                .into_response());
        }
    };
    let file = s
        .disk(move |store| seek_range(store.open(&id)?, start))
        .await?;
    if file.metadata().map_err(io_error)?.len() != bytes {
        return Err(error(ErrorCode::Internal));
    }
    tx.commit().await?;
    let reader = tokio::fs::File::from_std(file).take(end - start + 1);
    // Retain a concurrency permit for the stream's entire lifetime.
    let stream = tokio_util::io::ReaderStream::new(reader).map(move |chunk| {
        let _ = &permit;
        chunk
    });
    let mut response = Body::from_stream(stream).into_response();
    *response.status_mut() = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let headers = response.headers_mut();
    headers.insert("content-type", "application/octet-stream".parse().unwrap());
    // Display filename is in authorized metadata. A fixed ASCII header avoids
    // CRLF, quoting and Unicode interpretation ambiguities across user agents.
    headers.insert(
        "content-disposition",
        "attachment; filename=\"porch-download\"".parse().unwrap(),
    );
    headers.insert("accept-ranges", "bytes".parse().unwrap());
    headers.insert(
        "content-length",
        (end - start + 1).to_string().parse().unwrap(),
    );
    if range.is_some() {
        headers.insert(
            "content-range",
            format!("bytes {start}-{end}/{bytes}").parse().unwrap(),
        );
    }
    Ok(response)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_and_hashes_are_display_only() {
        for name in [
            "../secret",
            "/secret",
            "x\\y",
            "x\r\nLocation: evil",
            "\0",
            ".",
            "..",
            " ",
        ] {
            assert!(!valid_filename(name));
        }
        for name in ["hello world.png", "猫.txt", "<script>.html"] {
            assert!(valid_filename(name));
        }
        assert!(valid_hash(&"a".repeat(64)));
        assert!(!valid_hash(&"A".repeat(64)));
        assert!(!valid_hash(&"f".repeat(63)));
    }
}
