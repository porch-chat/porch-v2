//! Shared wire contracts. Define contracts here before implementing consumers.
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

/// Process liveness only. Database readiness is deliberately not implied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub protocol_version: u16,
}

impl HealthResponse {
    pub fn alive() -> Self {
        Self {
            status: "ok".into(),
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

/// Passwords and bearer credentials intentionally do not implement Debug.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub invite_code: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub admin: bool,
    pub can_invite: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub user: User,
    /// Returned once, never persisted in plaintext. Send as Authorization: Bearer.
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub created_at: String,
    pub expires_at: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInviteRequest {
    pub max_uses: i32,
    pub expires_in_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub id: i64,
    pub creator_id: i64,
    pub max_uses: i32,
    pub uses: i32,
    pub expires_at: String,
    pub revoked: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CreatedInvite {
    pub invite: Invite,
    /// Shown once. Listing never discloses credentials.
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    Forbidden,
    Conflict,
    InvalidInvite,
    NotFound,
    RateLimited,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: ErrorCode,
}
