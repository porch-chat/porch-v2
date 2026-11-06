//! Explicit authenticated media actions. No URL comes from message text; bearer
//! credentials stay in the worker and original files never open automatically.
use crate::{Engine, Error, Result, transfers::valid_id};
use futures_util::StreamExt;
use porch_protocol::LinkEmbed;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

struct PartialFile(Option<PathBuf>);
impl Drop for PartialFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Engine {
    pub async fn attachment_preview(
        &self,
        conversation: i64,
        message: i64,
        id: &str,
    ) -> Result<Vec<u8>> {
        self.active()?;
        if !valid_id(id) || conversation <= 0 || message <= 0 {
            return Err(Error::InvalidData);
        }
        let result = async {
            let response = self
                .inner
                .api
                .client
                .get(format!(
                    "{}/attachments/{id}/preview",
                    self.inner.api.origin()
                ))
                .bearer_auth(self.inner.api.token())
                .send()
                .await
                .map_err(|_| Error::Transport)?;
            if !response.status().is_success() {
                return Err(Error::Http(response.status().as_u16(), None));
            }
            if response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                != Some("image/png")
                || response
                    .content_length()
                    .is_some_and(|n| n > 2 * 1024 * 1024)
            {
                return Err(Error::InvalidData);
            }
            let mut output = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| Error::Transport)?;
                if output.len() + chunk.len() > 2 * 1024 * 1024 {
                    return Err(Error::InvalidData);
                }
                output.extend_from_slice(&chunk);
            }
            // The server re-encodes thumbnails, but an untrusted server must not
            // sneak SVG/HTML through a declared PNG content type.
            if !output.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(Error::InvalidData);
            }
            tokio::task::spawn_blocking(move || {
                let mut reader = image::ImageReader::with_format(
                    std::io::Cursor::new(&output),
                    image::ImageFormat::Png,
                );
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(480);
                limits.max_image_height = Some(320);
                limits.max_alloc = Some(4 * 1024 * 1024);
                reader.limits(limits);
                reader.decode().map_err(|_| Error::InvalidData)?;
                Ok(output)
            })
            .await
            .map_err(|_| Error::Storage)?
        }
        .await;
        self.transfer_result(result).await
    }
    pub async fn link_embed(
        &self,
        conversation: i64,
        message: i64,
        url: String,
    ) -> Result<LinkEmbed> {
        self.active()?;
        if conversation <= 0 || message <= 0 || url.len() > 2048 {
            return Err(Error::InvalidData);
        }
        self.transfer_result(
            self.inner
                .api
                .post(
                    &format!("/conversations/{conversation}/messages/{message}/embed"),
                    &porch_protocol::EmbedRequest { url },
                )
                .await,
        )
        .await
    }
    /// Stream to an exclusively created, user-chosen file. A failed request,
    /// integrity mismatch or task cancellation removes the partial output.
    /// The cached message supplies expected bytes/hash, not HTTP filenames.
    pub async fn download_attachment(
        &self,
        conversation: i64,
        message: i64,
        id: &str,
        destination: impl AsRef<Path>,
    ) -> Result<()> {
        self.active()?;
        if !valid_id(id) {
            return Err(Error::InvalidData);
        }
        let attachment = self
            .inner
            .cache
            .messages(conversation, None, 200)
            .await?
            .into_iter()
            .find(|m| m.id == message && !m.deleted)
            .and_then(|m| m.attachments.into_iter().find(|a| a.id == id))
            .ok_or(Error::InvalidData)?;
        if !(1..=porch_protocol::MAX_UPLOAD_BYTES).contains(&attachment.bytes) {
            return Err(Error::InvalidData);
        }
        let destination = destination.as_ref().to_path_buf();
        let result = async {
            let response = self
                .inner
                .api
                .client
                .get(format!("{}/attachments/{id}", self.inner.api.origin()))
                .bearer_auth(self.inner.api.token())
                .send()
                .await
                .map_err(|_| Error::Transport)?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(Error::Http(response.status().as_u16(), None));
            }
            if response
                .content_length()
                .is_some_and(|n| n != attachment.bytes)
            {
                return Err(Error::InvalidData);
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options.open(&destination).map_err(|_| Error::Storage)?;
            let mut partial = PartialFile(Some(destination));
            let mut file = tokio::fs::File::from_std(file);
            let mut hash = Sha256::new();
            let mut total = 0;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                self.active()?;
                let chunk = chunk.map_err(|_| Error::Transport)?;
                total += chunk.len() as u64;
                if total > attachment.bytes {
                    return Err(Error::InvalidData);
                }
                hash.update(&chunk);
                file.write_all(&chunk).await.map_err(|_| Error::Storage)?;
            }
            if total != attachment.bytes || format!("{:x}", hash.finalize()) != attachment.sha256 {
                return Err(Error::InvalidData);
            }
            file.sync_all().await.map_err(|_| Error::Storage)?;
            drop(file);
            partial.0 = None;
            Ok(())
        }
        .await;
        self.transfer_result(result).await
    }
}
