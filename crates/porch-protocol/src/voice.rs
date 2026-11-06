//! Porch-owned voice control plane, independent of any media provider.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceChannel {
    pub community_id: i64,
    pub channel_id: i64,
}
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceFlags {
    pub muted: bool,
    pub deafened: bool,
    pub speaking: bool,
}
impl VoiceFlags {
    pub fn effective(self, can_speak: bool) -> Self {
        let muted = self.muted || self.deafened || !can_speak;
        Self {
            muted,
            deafened: self.deafened,
            speaking: self.speaking && !muted,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceJoin {
    pub channel: VoiceChannel,
}
/// Credentials intentionally have no Debug implementation.
#[derive(Clone, Serialize, Deserialize)]
pub struct VoiceTicket {
    pub lease: String,
    pub channel: VoiceChannel,
    pub transport: VoiceTransport,
    pub can_speak: bool,
    pub heartbeat_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceTransport {
    Livekit { url: String, token: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceUpdate {
    pub lease: String,
    pub flags: VoiceFlags,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceLeave {
    pub lease: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceParticipant {
    pub user_id: i64,
    pub flags: VoiceFlags,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSnapshot {
    pub channel: VoiceChannel,
    pub participants: Vec<VoiceParticipant>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deafen_and_permissions_dominate_speaking() {
        for muted in [false, true] {
            for deafened in [false, true] {
                for can_speak in [false, true] {
                    let f = VoiceFlags {
                        muted,
                        deafened,
                        speaking: true,
                    }
                    .effective(can_speak);
                    assert_eq!(f.muted, muted || deafened || !can_speak);
                    assert_eq!(f.speaking, !f.muted);
                }
            }
        }
    }
}
