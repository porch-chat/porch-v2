//! Community wire contracts. IDs are community-local except CommunitySummary::id.
use serde::{Deserialize, Serialize};

/// Integer bitflags, restricted to ALL. JSON numbers remain exactly representable.
pub mod permissions {
    pub const VIEW_CHANNEL: u64 = 1 << 0;
    pub const SEND_MESSAGES: u64 = 1 << 1;
    pub const READ_HISTORY: u64 = 1 << 2;
    pub const ATTACH_FILES: u64 = 1 << 3;
    pub const MENTION_EVERYONE: u64 = 1 << 4;
    pub const MANAGE_MESSAGES: u64 = 1 << 5;
    pub const MANAGE_CHANNELS: u64 = 1 << 6;
    pub const MANAGE_ROLES: u64 = 1 << 7;
    pub const KICK_MEMBERS: u64 = 1 << 8;
    pub const BAN_MEMBERS: u64 = 1 << 9;
    pub const CREATE_INVITE: u64 = 1 << 10;
    pub const CONNECT: u64 = 1 << 11;
    pub const SPEAK: u64 = 1 << 12;
    pub const STREAM: u64 = 1 << 13;
    pub const USE_VAD: u64 = 1 << 14;
    pub const MANAGE_COMMUNITY: u64 = 1 << 15;
    pub const ADMINISTRATOR: u64 = 1 << 16;
    pub const ALL: u64 = (1 << 17) - 1;
    pub const DEFAULT: u64 = VIEW_CHANNEL
        | SEND_MESSAGES
        | READ_HISTORY
        | ATTACH_FILES
        | CONNECT
        | SPEAK
        | USE_VAD
        | CREATE_INVITE;
    pub const SEND_DEPENDENT: u64 = ATTACH_FILES | MENTION_EVERYONE;
    pub const CONNECT_DEPENDENT: u64 = SPEAK | STREAM | USE_VAD;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySummary {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
    pub official: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCommunity {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    /// Zero is the immutable @everyone identity.
    pub id: i64,
    pub name: String,
    pub permissions: u64,
    /// Dense ordering; higher is stronger. @everyone is always zero.
    pub position: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: i64,
    /// @everyone membership is implicit, never stored here.
    pub role_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum OverwriteTarget {
    Everyone,
    Role(i64),
    Member(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionOverwrite {
    pub target: OverwriteTarget,
    pub allow: u64,
    pub deny: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub position: usize,
    pub overwrites: Vec<PermissionOverwrite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub kind: ChannelKind,
    pub category_id: Option<i64>,
    /// Dense sibling order, independently within each category / uncategorized.
    pub position: usize,
    /// When true, resolve category overwrites dynamically. No local overwrites.
    pub synced: bool,
    pub overwrites: Vec<PermissionOverwrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunitySnapshot {
    pub community: CommunitySummary,
    pub roles: Vec<Role>,
    pub members: Vec<Member>,
    /// Only categories with at least one visible channel, or visible empty categories.
    pub categories: Vec<Category>,
    pub channels: Vec<Channel>,
    /// Only returned to members with BAN_MEMBERS.
    pub bans: Vec<i64>,
    pub permissions: u64,
    /// Display names for listed members, filled by the server when serving a snapshot.
    #[serde(default)]
    pub usernames: std::collections::BTreeMap<i64, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommunityCommand {
    Rename {
        name: String,
    },
    TransferOwnership {
        user_id: i64,
    },
    Delete,
    CreateRole {
        name: String,
        permissions: u64,
        position: usize,
    },
    EditRole {
        id: i64,
        name: String,
        permissions: u64,
        position: usize,
    },
    DeleteRole {
        id: i64,
    },
    SetMemberRoles {
        user_id: i64,
        role_ids: Vec<i64>,
    },
    CreateCategory {
        name: String,
        position: usize,
    },
    EditCategory {
        id: i64,
        name: String,
        position: usize,
    },
    DeleteCategory {
        id: i64,
    },
    CreateChannel {
        name: String,
        kind: ChannelKind,
        category_id: Option<i64>,
        position: usize,
        synced: bool,
    },
    EditChannel {
        id: i64,
        name: String,
        category_id: Option<i64>,
        position: usize,
        synced: bool,
    },
    DeleteChannel {
        id: i64,
    },
    SetOverwrites {
        channel_id: Option<i64>,
        category_id: Option<i64>,
        overwrites: Vec<PermissionOverwrite>,
    },
    Leave,
    Kick {
        user_id: i64,
    },
    Ban {
        user_id: i64,
    },
    Unban {
        user_id: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMutation {
    pub id: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinCommunity {
    pub code: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CreatedCommunityInvite {
    pub id: i64,
    pub community_id: i64,
    /// Returned once. Never included in snapshots, lists, or gateway events.
    pub code: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityInvite {
    pub id: i64,
    pub creator_id: i64,
    pub max_uses: i32,
    pub uses: i32,
    pub expires_at: String,
    pub revoked: bool,
}
