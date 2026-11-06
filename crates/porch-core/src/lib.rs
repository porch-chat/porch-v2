//! Account-bound client engine shared by terminal and native clients.
mod cache;
mod engine;
mod gateway;
mod transfers;
mod transport;
pub use cache::{Cache, PendingSend, RemovalNotice, RemovedLocalSend, SendState};
pub use engine::Engine;
pub use gateway::{ConnectionState, GatewayTask};
use porch_protocol::ErrorCode;
pub use transfers::PendingUpload;
pub use transport::Api;

pub fn protocol_version() -> u16 {
    porch_protocol::PROTOCOL_VERSION
}

/// Intentionally contains no URLs, tokens, source request or server response body.
#[derive(Debug)]
pub enum Error {
    Transport,
    Http(u16, Option<ErrorCode>),
    Storage,
    InvalidData,
    AccountMismatch,
    SignedOut { discarded_sends: usize },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "client error {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidData
    }
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn unauthorized(&self) -> bool {
        matches!(self, Self::Http(401, _) | Self::SignedOut { .. })
    }
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport | Self::Http(429, _) | Self::Http(500..=599, _)
        )
    }
    pub fn inaccessible(&self) -> bool {
        matches!(self, Self::Http(403 | 404, _))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_contract() {
        assert_eq!(super::protocol_version(), porch_protocol::PROTOCOL_VERSION);
    }
}
