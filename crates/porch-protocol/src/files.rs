//! File transfers use raw bounded chunk bodies, never JSON/base64 gigabytes.
use serde::{Deserialize, Serialize};

pub const MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
pub const UPLOAD_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUpload {
    pub conversation_id: i64,
    pub filename: String,
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the entire original.
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadState {
    Receiving,
    Ready,
    Attached,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadStatus {
    pub id: String,
    pub conversation_id: i64,
    pub filename: String,
    pub bytes: u64,
    pub received: u64,
    pub chunk_bytes: u64,
    pub sha256: String,
    pub state: UploadState,
    pub expires_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    /// Detected by server, not supplied by the uploader.
    pub mime: String,
}
