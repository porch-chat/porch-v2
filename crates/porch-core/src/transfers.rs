//! Resumable uploads keep only opaque ID, private source path and acknowledged
//! status in the account-bound cache, never the bearer token or file bytes.
use crate::{Engine, Error, Result};
use porch_protocol::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpload {
    pub source: PathBuf,
    pub status: UploadStatus,
}
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn validate(s: &UploadStatus) -> Result<()> {
    if !valid_id(&s.id)
        || s.chunk_bytes != UPLOAD_CHUNK_BYTES
        || s.received > s.bytes
        || !(1..=MAX_UPLOAD_BYTES).contains(&s.bytes)
        || s.sha256.len() != 64
        || !s
            .sha256
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || (s.received != s.bytes && !s.received.is_multiple_of(UPLOAD_CHUNK_BYTES))
    {
        return Err(Error::InvalidData);
    }
    Ok(())
}
async fn inspect(path: PathBuf) -> Result<CreateUpload> {
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&path).map_err(|_| Error::Storage)?;
        let m = file.metadata().map_err(|_| Error::Storage)?;
        if !m.is_file() || !(1..=MAX_UPLOAD_BYTES).contains(&m.len()) {
            return Err(Error::InvalidData);
        }
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(Error::InvalidData)?
            .to_owned();
        let mut h = Sha256::new();
        let mut buf = [0; 65536];
        let mut bytes = 0;
        loop {
            let n = file.read(&mut buf).map_err(|_| Error::Storage)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > MAX_UPLOAD_BYTES {
                return Err(Error::InvalidData);
            }
            h.update(&buf[..n]);
        }
        if bytes != m.len() {
            return Err(Error::InvalidData);
        }
        Ok(CreateUpload {
            conversation_id: 0,
            filename,
            bytes,
            sha256: format!("{:x}", h.finalize()),
        })
    })
    .await
    .map_err(|_| Error::Storage)?
}
impl Engine {
    async fn transfer_result<T>(&self, result: Result<T>) -> Result<T> {
        if result.as_ref().err().is_some_and(Error::unauthorized) {
            self.expire().await?;
        }
        result
    }
    async fn save_upload(&self, upload: &PendingUpload) -> Result<()> {
        self.active()?;
        sqlx::query("INSERT INTO transfers(id,conversation_id,data) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data")
            .bind(&upload.status.id).bind(upload.status.conversation_id).bind(serde_json::to_string(upload)?)
            .execute(&self.inner.cache.pool).await?;
        Ok(())
    }
    pub async fn pending_uploads(&self) -> Result<Vec<PendingUpload>> {
        self.active()?;
        let rows: Vec<(String,)> = sqlx::query_as("SELECT data FROM transfers ORDER BY id")
            .fetch_all(&self.inner.cache.pool)
            .await?;
        rows.into_iter()
            .map(|(s,)| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    /// A separate explicit action sends chunks; choosing a file cannot silently
    /// transfer its contents. The cached source path is never included in HTTP.
    pub async fn begin_upload(
        &self,
        conversation_id: i64,
        path: impl AsRef<Path>,
    ) -> Result<PendingUpload> {
        self.active()?;
        let source = path.as_ref().to_path_buf();
        let mut request = inspect(source.clone()).await?;
        request.conversation_id = conversation_id;
        // Require an already synchronized visible conversation locally, in
        // addition to authoritative server authorization.
        if !self
            .inner
            .cache
            .heads()
            .await?
            .iter()
            .any(|h| h.id == conversation_id)
        {
            return Err(Error::InvalidData);
        }
        let status: UploadStatus = self
            .transfer_result(self.inner.api.post("/uploads", &request).await)
            .await?;
        validate(&status)?;
        if status.conversation_id != conversation_id
            || status.sha256 != request.sha256
            || status.bytes != request.bytes
        {
            return Err(Error::InvalidData);
        }
        let upload = PendingUpload { source, status };
        self.save_upload(&upload).await?;
        Ok(upload)
    }
    /// Re-query the authoritative committed offset on EVERY invocation. One
    /// invocation sends at most one chunk, making progress/cancel/reconnect
    /// explicit and bounding native UI work. Failed requests are never guessed.
    pub async fn advance_upload(&self, id: &str) -> Result<PendingUpload> {
        self.active()?;
        if !valid_id(id) {
            return Err(Error::InvalidData);
        }
        let saved = self
            .pending_uploads()
            .await?
            .into_iter()
            .find(|u| u.status.id == id)
            .ok_or(Error::InvalidData)?;
        let mut upload = saved.clone();
        let status: UploadStatus = self
            .transfer_result(self.inner.api.get(&format!("/uploads/{id}")).await)
            .await?;
        validate(&status)?;
        if status.id != id
            || status.conversation_id != saved.status.conversation_id
            || status.sha256 != saved.status.sha256
            || status.bytes != saved.status.bytes
        {
            return Err(Error::InvalidData);
        }
        upload.status = status;
        // Rehash before each resumed chunk is deliberate v1 safety for edited
        // source files; memory remains bounded, latency tradeoff is documented.
        let source = inspect(upload.source.clone()).await?;
        if source.bytes != upload.status.bytes || source.sha256 != upload.status.sha256 {
            return Err(Error::InvalidData);
        }
        self.save_upload(&upload).await?;
        if upload.status.state == UploadState::Receiving {
            if upload.status.received < upload.status.bytes {
                let path = upload.source.clone();
                let offset = upload.status.received;
                let len = (upload.status.bytes - offset).min(UPLOAD_CHUNK_BYTES) as usize;
                let chunk = tokio::task::spawn_blocking(move || {
                    let mut file = std::fs::File::open(path).map_err(|_| Error::Storage)?;
                    file.seek(SeekFrom::Start(offset))
                        .map_err(|_| Error::Storage)?;
                    let mut data = vec![0; len];
                    file.read_exact(&mut data).map_err(|_| Error::Storage)?;
                    Ok::<_, Error>(data)
                })
                .await
                .map_err(|_| Error::Storage)??;
                let result = async {
                    let response = self
                        .inner
                        .api
                        .client
                        .post(format!(
                            "{}/uploads/{id}/chunks?offset={offset}",
                            self.inner.api.origin()
                        ))
                        .bearer_auth(self.inner.api.token())
                        .body(chunk)
                        .send()
                        .await
                        .map_err(|_| Error::Transport)?;
                    let code = response.status();
                    if !code.is_success() {
                        return Err(Error::Http(code.as_u16(), None));
                    }
                    // Status is tiny, but do not trust a malicious origin to
                    // return an unbounded body to the native application.
                    use futures_util::StreamExt;
                    let mut stream = response.bytes_stream();
                    let mut data = vec![];
                    while let Some(part) = stream.next().await {
                        let part = part.map_err(|_| Error::Transport)?;
                        if data.len() + part.len() > 8192 {
                            return Err(Error::InvalidData);
                        }
                        data.extend_from_slice(&part);
                    }
                    Ok::<UploadStatus, Error>(serde_json::from_slice(&data)?)
                }
                .await;
                let next = self.transfer_result(result).await?;
                validate(&next)?;
                if next.id != id
                    || next.sha256 != upload.status.sha256
                    || next.bytes != upload.status.bytes
                    || next.conversation_id != upload.status.conversation_id
                    || next.received != offset + len as u64
                {
                    return Err(Error::InvalidData);
                }
                upload.status = next;
            } else {
                let next: UploadStatus = self
                    .transfer_result(
                        self.inner
                            .api
                            .post(&format!("/uploads/{id}/complete"), &())
                            .await,
                    )
                    .await?;
                validate(&next)?;
                if next.id != id
                    || next.sha256 != upload.status.sha256
                    || next.conversation_id != upload.status.conversation_id
                    || next.bytes != upload.status.bytes
                    || next.state != UploadState::Ready
                {
                    return Err(Error::InvalidData);
                }
                upload.status = next;
            }
            self.save_upload(&upload).await?;
        }
        Ok(upload)
    }
    pub async fn cancel_upload(&self, id: &str) -> Result<()> {
        self.active()?;
        if !valid_id(id) {
            return Err(Error::InvalidData);
        }
        // Only locally owned transfers; no generic deletion of guessed IDs.
        if !self
            .pending_uploads()
            .await?
            .iter()
            .any(|u| u.status.id == id)
        {
            return Err(Error::InvalidData);
        }
        let result = self
            .inner
            .api
            .bytes::<()>(reqwest::Method::DELETE, &format!("/uploads/{id}"), None)
            .await;
        match self.transfer_result(result).await {
            Ok(_) => {}
            Err(e) if e.inaccessible() => {}
            Err(e) => return Err(e),
        }
        sqlx::query("DELETE FROM transfers WHERE id=?")
            .bind(id)
            .execute(&self.inner.cache.pool)
            .await?;
        Ok(())
    }
}
