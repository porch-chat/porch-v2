use crate::{Error, Result};
use porch_protocol::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendState {
    Queued,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSend {
    pub conversation_id: i64,
    pub request: SendMessage,
    pub state: SendState,
    pub failure: Option<String>,
}
/// Local rows removed, not proof that a request was never delivered remotely.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedLocalSend {
    pub conversation_id: i64,
    pub nonce: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovalNotice {
    pub removed_at_unix_seconds: u64,
    pub sends: Vec<RemovedLocalSend>,
    pub delivery_may_have_succeeded: bool,
}
#[derive(Clone)]
pub struct Cache {
    pub(crate) pool: SqlitePool,
    pub(crate) user: i64,
}
fn json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn parse<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_str(text)?)
}
impl Cache {
    pub async fn open(path: impl AsRef<Path>, origin: &str, user: i64) -> Result<Self> {
        if user <= 0 {
            return Err(Error::InvalidData);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path.as_ref())
            {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(Error::Storage),
            }
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS binding(id INTEGER PRIMARY KEY CHECK(id=1),origin TEXT NOT NULL,user_id INTEGER NOT NULL,version INTEGER NOT NULL CHECK(version=1),profile TEXT,removal_notice TEXT);
        CREATE TABLE IF NOT EXISTS conversations(id INTEGER PRIMARY KEY,data TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY,conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,revision INTEGER NOT NULL,data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id,id);
        CREATE TABLE IF NOT EXISTS outbox(id INTEGER PRIMARY KEY AUTOINCREMENT,conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,nonce TEXT NOT NULL UNIQUE,data TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('queued','failed')),failure TEXT);
        CREATE TABLE IF NOT EXISTS communities(id INTEGER PRIMARY KEY,data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS transfers(id TEXT PRIMARY KEY,conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,data TEXT NOT NULL);").execute(&pool).await?;
        let mut tx = pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO binding(id,origin,user_id,version) VALUES(1,?,?,1)")
            .bind(origin)
            .bind(user)
            .execute(&mut *tx)
            .await?;
        let (bound_origin, bound_user, version): (String, i64, i64) =
            sqlx::query_as("SELECT origin,user_id,version FROM binding WHERE id=1")
                .fetch_one(&mut *tx)
                .await?;
        if bound_origin != origin || bound_user != user {
            return Err(Error::AccountMismatch);
        }
        if version != 1 {
            return Err(Error::InvalidData);
        }
        tx.commit().await?;
        Ok(Self { pool, user })
    }
    pub(crate) async fn save_user(&self, user: &User) -> Result<()> {
        if user.id != self.user {
            return Err(Error::AccountMismatch);
        }
        sqlx::query("UPDATE binding SET profile=? WHERE id=1")
            .bind(json(user)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub(crate) async fn restore(path: impl AsRef<Path>, origin: &str) -> Result<(Self, User)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await?;
        let (bound, user, profile): (String, i64, Option<String>) =
            sqlx::query_as("SELECT origin,user_id,profile FROM binding WHERE id=1 AND version=1")
                .fetch_one(&pool)
                .await?;
        if bound != origin {
            return Err(Error::AccountMismatch);
        }
        let profile: User = parse(profile.as_deref().ok_or(Error::InvalidData)?)?;
        if profile.id != user {
            return Err(Error::InvalidData);
        }
        Ok((Self { pool, user }, profile))
    }
    /// Durable latest nonempty local-removal notice. Acknowledge after displaying.
    pub async fn removal_notice(&self) -> Result<Option<RemovalNotice>> {
        let notice: Option<String> =
            sqlx::query_scalar("SELECT removal_notice FROM binding WHERE id=1")
                .fetch_one(&self.pool)
                .await?;
        notice.as_deref().map(parse).transpose()
    }
    pub async fn acknowledge_removal_notice(&self) -> Result<()> {
        sqlx::query("UPDATE binding SET removal_notice=NULL WHERE id=1")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn clear(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT conversation_id,nonce FROM outbox ORDER BY id")
                .fetch_all(&mut *tx)
                .await?;
        if !rows.is_empty() {
            let notice = RemovalNotice {
                removed_at_unix_seconds: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| Error::InvalidData)?
                    .as_secs(),
                sends: rows
                    .into_iter()
                    .map(|(conversation_id, nonce)| RemovedLocalSend {
                        conversation_id,
                        nonce,
                    })
                    .collect(),
                delivery_may_have_succeeded: true,
            };
            sqlx::query("UPDATE binding SET removal_notice=? WHERE id=1")
                .bind(json(&notice)?)
                .execute(&mut *tx)
                .await?;
        }
        for statement in [
            "DELETE FROM outbox",
            "DELETE FROM messages",
            "DELETE FROM conversations",
            "DELETE FROM communities",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn heads(&self) -> Result<Vec<ConversationHead>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT data FROM conversations ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|v| parse(v)).collect()
    }
    pub async fn communities(&self) -> Result<Vec<CommunitySnapshot>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT data FROM communities ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|v| parse(v)).collect()
    }
    pub(crate) async fn replace_communities(&self, values: &[CommunitySnapshot]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM communities")
            .execute(&mut *tx)
            .await?;
        for c in values {
            sqlx::query("INSERT INTO communities(id,data) VALUES(?,?)")
                .bind(c.community.id)
                .bind(json(c)?)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Only call with a complete authoritative head response. Missing IDs cascade purge.
    pub(crate) async fn replace_heads(&self, heads: &[ConversationHead]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let old: Vec<i64> = sqlx::query_scalar("SELECT id FROM conversations")
            .fetch_all(&mut *tx)
            .await?;
        for id in old {
            if !heads.iter().any(|h| h.id == id) {
                sqlx::query("DELETE FROM conversations WHERE id=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        for h in heads {
            sqlx::query("INSERT INTO conversations(id,data) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data").bind(h.id).bind(json(h)?).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn revision(&self, id: i64) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT revision FROM conversations WHERE id=?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub(crate) async fn purge(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM conversations WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub(crate) async fn reset_messages(&self, id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM messages WHERE conversation_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE conversations SET revision=0 WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn apply(
        &self,
        id: i64,
        messages: &[Message],
        cursor: Option<i64>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for m in messages {
            if m.conversation_id != id || m.id <= 0 || m.revision <= 0 {
                return Err(Error::InvalidData);
            }
            sqlx::query("INSERT INTO messages(id,conversation_id,revision,data) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data WHERE messages.conversation_id=excluded.conversation_id AND excluded.revision>=messages.revision").bind(m.id).bind(id).bind(m.revision).bind(json(m)?).execute(&mut *tx).await?;
            if m.author_id == self.user {
                sqlx::query("DELETE FROM outbox WHERE conversation_id=? AND nonce=?")
                    .bind(id)
                    .bind(&m.nonce)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if let Some(cursor) = cursor {
            sqlx::query("UPDATE conversations SET revision=? WHERE id=?")
                .bind(cursor)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn messages(&self, id: i64, before: Option<i64>, limit: u32) -> Result<Vec<Message>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidData);
        }
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT data FROM messages WHERE conversation_id=? AND id<? ORDER BY id DESC LIMIT ?",
        )
        .bind(id)
        .bind(before.unwrap_or(i64::MAX))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|v| parse(v)).collect()
    }
    pub(crate) async fn enqueue(&self, id: i64, request: &SendMessage) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
            .fetch_one(&mut *tx)
            .await?;
        if count >= 500 {
            return Err(Error::InvalidData);
        }
        sqlx::query("INSERT INTO outbox(conversation_id,nonce,data,state) VALUES(?,?,?,'queued')")
            .bind(id)
            .bind(&request.nonce)
            .bind(json(request)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn pending(&self) -> Result<Vec<PendingSend>> {
        let rows = sqlx::query("SELECT conversation_id,data,state,failure FROM outbox ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| {
                Ok(PendingSend {
                    conversation_id: r.get(0),
                    request: parse(r.get(1))?,
                    state: if r.get::<String, _>(2) == "queued" {
                        SendState::Queued
                    } else {
                        SendState::Failed
                    },
                    failure: r.get(3),
                })
            })
            .collect()
    }
    pub(crate) async fn fail(&self, nonce: &str, status: u16) -> Result<()> {
        sqlx::query("UPDATE outbox SET state='failed',failure=? WHERE nonce=?")
            .bind(format!("HTTP {status}"))
            .bind(nonce)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub(crate) async fn retry(&self, nonce: &str) -> Result<()> {
        sqlx::query("UPDATE outbox SET state='queued',failure=NULL WHERE nonce=?")
            .bind(nonce)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub(crate) async fn discard(&self, nonce: &str) -> Result<()> {
        sqlx::query("DELETE FROM outbox WHERE nonce=?")
            .bind(nonce)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    fn head(id: i64) -> ConversationHead {
        ConversationHead {
            id,
            community_id: None,
            channel_id: None,
            name: "DM".into(),
            owner_id: Some(1),
            participants: vec![1, 2],
            revision: 10,
            last_message_id: Some(1),
            read_message_id: 0,
            unread_count: 1,
            mention_count: 0,
        }
    }
    fn message(revision: i64) -> Message {
        Message {
            attachments: vec![],
            id: 1,
            conversation_id: 1,
            author_id: 1,
            nonce: "nonce_test_0000001".into(),
            content: if revision == 10 {
                None
            } else {
                Some(MessageContent::Markdown {
                    text: format!("r{revision}"),
                })
            },
            rendered_html: None,
            reply_to: None,
            mentions: vec![],
            mention_everyone: false,
            created_at: "now".into(),
            edited_at: None,
            deleted: revision == 10,
            pinned: false,
            reactions: vec![],
            revision,
        }
    }
    fn request() -> SendMessage {
        SendMessage {
            attachments: vec![],
            nonce: "nonce_test_0000001".into(),
            content: MessageContent::Markdown {
                text: "hello".into(),
            },
            reply_to: None,
            mentions: vec![],
            mention_everyone: false,
        }
    }
    #[tokio::test]
    async fn account_binding_purge_and_durable_outbox() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let cache = Cache::open(&path, "https://porch.test", 1).await.unwrap();
        cache.replace_heads(&[head(1), head(2)]).await.unwrap();
        cache.enqueue(1, &request()).await.unwrap();
        assert!(matches!(
            Cache::open(&path, "https://elsewhere.test", 1).await,
            Err(Error::AccountMismatch)
        ));
        assert!(matches!(
            Cache::open(&path, "https://porch.test", 2).await,
            Err(Error::AccountMismatch)
        ));
        drop(cache);
        let cache = Cache::open(&path, "https://porch.test", 1).await.unwrap();
        assert_eq!(cache.pending().await.unwrap().len(), 1);
        cache.fail(&request().nonce, 400).await.unwrap();
        assert_eq!(cache.pending().await.unwrap()[0].state, SendState::Failed);
        cache.retry(&request().nonce).await.unwrap();
        assert_eq!(cache.pending().await.unwrap()[0].state, SendState::Queued);
        cache.replace_heads(&[head(2)]).await.unwrap();
        assert!(cache.pending().await.unwrap().is_empty());
        assert!(cache.messages(1, None, 10).await.unwrap().is_empty());
        assert!(cache.heads().await.unwrap().iter().all(|h| h.id == 2));
        cache.clear().await.unwrap();
        assert!(cache.heads().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn revision_ack_cursor_and_transaction_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().join("db"), "https://porch.test", 1)
            .await
            .unwrap();
        cache.replace_heads(&[head(1)]).await.unwrap();
        cache.enqueue(1, &request()).await.unwrap();
        let mut other = message(3);
        other.author_id = 2;
        cache.apply(1, &[other], None).await.unwrap();
        assert_eq!(cache.pending().await.unwrap().len(), 1);
        cache.apply(1, &[message(3)], None).await.unwrap();
        assert!(cache.pending().await.unwrap().is_empty());
        assert_eq!(cache.revision(1).await.unwrap(), 0); // ack never advances cursor
        cache.apply(1, &[message(10)], Some(10)).await.unwrap();
        cache.apply(1, &[message(2)], None).await.unwrap();
        assert!(cache.messages(1, None, 10).await.unwrap()[0].deleted);
        let mut bad = message(12);
        bad.conversation_id = 2;
        assert!(cache.apply(1, &[message(11), bad], Some(12)).await.is_err());
        assert_eq!(cache.messages(1, None, 10).await.unwrap()[0].revision, 10);
        assert_eq!(cache.revision(1).await.unwrap(), 10);
        cache.reset_messages(1).await.unwrap();
        assert_eq!(cache.revision(1).await.unwrap(), 0);
        assert!(cache.messages(1, None, 10).await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn removal_notice_survives_crash_boundary_without_draft_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let cache = Cache::open(&path, "https://porch.test", 1).await.unwrap();
        cache.replace_heads(&[head(1)]).await.unwrap();
        cache.enqueue(1, &request()).await.unwrap();
        cache.clear().await.unwrap();
        drop(cache);
        let cache = Cache::open(&path, "https://porch.test", 1).await.unwrap();
        let notice = cache.removal_notice().await.unwrap().unwrap();
        assert_eq!(notice.sends.len(), 1);
        assert_eq!(notice.sends[0].conversation_id, 1);
        assert!(notice.delivery_may_have_succeeded);
        assert!(!serde_json::to_string(&notice).unwrap().contains("hello"));
        cache.clear().await.unwrap();
        assert!(cache.removal_notice().await.unwrap().is_some());
        cache.acknowledge_removal_notice().await.unwrap();
        assert!(cache.removal_notice().await.unwrap().is_none());
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]fn arbitrarily_reordered_revisions_never_resurrect(versions in prop::collection::vec(1i64..=10,1..40)){
            let rt=tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async{
                let dir=tempfile::tempdir().unwrap();let cache=Cache::open(dir.path().join("db"),"https://porch.test",1).await.unwrap();
                cache.replace_heads(&[head(1)]).await.unwrap();
                cache.apply(1,&[message(10)],None).await.unwrap();
                for revision in versions{cache.apply(1,&[message(revision)],None).await.unwrap();}
                let result=cache.messages(1,None,10).await.unwrap();assert_eq!(result.len(),1);assert_eq!(result[0].revision,10);assert!(result[0].deleted);assert!(result[0].content.is_none());
            });
        }
    }
}
