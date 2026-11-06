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
