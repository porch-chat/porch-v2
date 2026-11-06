//! Conversation IDs are global; channel IDs remain community-local.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageContent {
    Markdown {
        text: String,
    },
    /// Reserved seam. Servers currently reject this, never imply E2EE support.
    Ciphertext {
        algorithm: String,
        payload: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    pub nonce: String,
    pub content: MessageContent,
    pub reply_to: Option<i64>,
    #[serde(default)]
    pub mentions: Vec<i64>,
    #[serde(default)]
    pub mention_everyone: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub attachments: Vec<crate::Attachment>,
    pub id: i64,
    pub conversation_id: i64,
    pub author_id: i64,
    pub nonce: String,
    pub content: Option<MessageContent>,
    /// Sanitized HTML, never raw source HTML. Native clients may render source as text.
    pub rendered_html: Option<String>,
    pub reply_to: Option<i64>,
    pub mentions: Vec<i64>,
    pub mention_everyone: bool,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted: bool,
    pub pinned: bool,
    pub reactions: Vec<Reaction>,
    pub revision: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageCommand {
    Edit {
        content: MessageContent,
        mentions: Vec<i64>,
        mention_everyone: bool,
    },
    Delete,
    React {
        emoji: String,
        active: bool,
    },
    Pin {
        pinned: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationHead {
    pub id: i64,
    pub community_id: Option<i64>,
    pub channel_id: Option<i64>,
    pub name: String,
    pub owner_id: Option<i64>,
    /// Populated only for standalone DMs.
    pub participants: Vec<i64>,
    pub revision: i64,
    pub last_message_id: Option<i64>,
    pub read_message_id: i64,
    pub unread_count: i64,
    pub mention_count: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDm {
    /// Other users; the actor is added automatically. One peer creates/reopens 1:1.
    pub participants: Vec<i64>,
    pub name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DmCommand {
    Rename { name: String },
    Add { user_id: i64 },
    Remove { user_id: i64 },
    TransferOwner { user_id: i64 },
    Leave,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkRead {
    pub message_id: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    /// Next older ID for history, or latest revision for delta pages.
    pub next: Option<i64>,
    pub high_watermark: i64,
}
