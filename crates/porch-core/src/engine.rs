use crate::{Api, Cache, Error, Result, SendState};
use porch_protocol::*;
use rand::{RngCore, rngs::OsRng};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}
pub(crate) struct Inner {
    pub api: Api,
    pub cache: Cache,
    pub user: User,
    pub gate: Mutex<()>,
    pub signed_out: AtomicBool,
    pub gateway_active: AtomicBool,
}
impl Engine {
    pub async fn open(origin: &str, token: String, path: impl AsRef<Path>) -> Result<Self> {
        let api = Api::new(origin, token)?;
        let (cache, user) = match api.get::<User>("/auth/me").await {
            Ok(user) => {
                let cache = Cache::open(&path, api.origin(), user.id).await?;
                cache.save_user(&user).await?;
                (cache, user)
            }
            Err(e) if e.retryable() => Cache::restore(&path, api.origin()).await?,
            Err(e) => {
                if e.unauthorized()
                    && let Ok((cache, _)) = Cache::restore(&path, api.origin()).await
                {
                    cache.clear().await?;
                }
                return Err(e);
            }
        };
        Ok(Self {
            inner: Arc::new(Inner {
                api,
                cache,
                user,
                gate: Mutex::new(()),
                signed_out: AtomicBool::new(false),
                gateway_active: AtomicBool::new(false),
            }),
        })
    }
    pub fn user(&self) -> &User {
        &self.inner.user
    }
    pub fn cache(&self) -> Cache {
        self.inner.cache.clone()
    }
    pub fn api(&self) -> Api {
        self.inner.api.clone()
    }
    pub fn is_signed_out(&self) -> bool {
        self.inner.signed_out.load(Ordering::SeqCst)
    }
    fn active(&self) -> Result<()> {
        if self.is_signed_out() {
            Err(Error::SignedOut)
        } else {
            Ok(())
        }
    }
    async fn checked<T>(&self, result: Result<T>) -> Result<T> {
        if result.as_ref().err().is_some_and(Error::unauthorized) {
            self.inner.signed_out.store(true, Ordering::SeqCst);
            self.inner.cache.clear().await?;
        }
        result
    }
    pub(crate) async fn expire(&self) -> Result<()> {
        let _lock = self.inner.gate.lock().await;
        self.inner.signed_out.store(true, Ordering::SeqCst);
        self.inner.cache.clear().await
    }
    pub async fn logout(&self) -> Result<()> {
        let _lock = self.inner.gate.lock().await;
        self.inner.signed_out.store(true, Ordering::SeqCst);
        let result = self.inner.api.post_empty("/auth/logout", &()).await;
        self.inner.cache.clear().await?;
        result
    }
    pub async fn reconcile(&self) -> Result<()> {
        let _lock = self.inner.gate.lock().await;
        self.active()?;
        self.checked(self.reconcile_locked().await).await
    }
    async fn reconcile_locked(&self) -> Result<()> {
        let user: User = self.inner.api.get("/auth/me").await?;
        if user.id != self.inner.user.id {
            self.inner.signed_out.store(true, Ordering::SeqCst);
            self.inner.cache.clear().await?;
            return Err(Error::AccountMismatch);
        }
        let heads: Vec<ConversationHead> = self.inner.api.get("/conversations").await?;
        if heads.iter().any(|h| h.id <= 0 || h.revision < 0)
            || heads
                .iter()
                .map(|h| h.id)
                .collect::<std::collections::HashSet<_>>()
                .len()
                != heads.len()
        {
            return Err(Error::InvalidData);
        }
        self.inner.cache.replace_heads(&heads).await?;
        for h in heads {
            let mut after = self.inner.cache.revision(h.id).await?;
            if h.revision < after {
                self.inner.cache.reset_messages(h.id).await?;
                after = 0;
            }
            if after == h.revision {
                continue;
            }
            loop {
                let page: MessagePage = match self
                    .inner
                    .api
                    .get(&format!(
                        "/conversations/{}/delta?after={after}&through={}&limit=100",
                        h.id, h.revision
                    ))
                    .await
                {
                    Ok(p) => p,
                    Err(e) if e.inaccessible() => {
                        self.inner.cache.purge(h.id).await?;
                        break;
                    }
                    Err(e) => return Err(e),
                };
                if page.high_watermark != h.revision
                    || page.messages.len() > 100
                    || page.messages.iter().any(|m| {
                        m.conversation_id != h.id || m.revision <= after || m.revision > h.revision
                    })
                    || page
                        .messages
                        .windows(2)
                        .any(|w| w[0].revision >= w[1].revision)
                {
                    return Err(Error::InvalidData);
                }
                if let Some(next) = page.next {
                    if next <= after
                        || next > h.revision
                        || page.messages.last().map(|m| m.revision) != Some(next)
                    {
                        return Err(Error::InvalidData);
                    }
                    self.inner.cache.apply(h.id, &page.messages, None).await?;
                    after = next;
                } else {
                    self.inner
                        .cache
                        .apply(h.id, &page.messages, Some(h.revision))
                        .await?;
                    break;
                }
            }
        }
        let communities: Vec<CommunitySummary> = self.inner.api.get("/communities").await?;
        let mut snapshots = vec![];
        for c in communities {
            match self.inner.api.get(&format!("/communities/{}", c.id)).await {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(e) if e.inaccessible() => {}
                Err(e) => return Err(e),
            }
        }
        self.inner.cache.replace_communities(&snapshots).await?;
        Ok(())
    }
    /// Durable optimistic send. This does not require a working network connection.
    pub async fn queue(
        &self,
        id: i64,
        content: MessageContent,
        reply_to: Option<i64>,
        mentions: Vec<i64>,
        mention_everyone: bool,
    ) -> Result<SendMessage> {
        let _lock = self.inner.gate.lock().await;
        self.active()?;
        if !matches!(&content,MessageContent::Markdown{text} if !text.trim().is_empty()&&text.chars().count()<=8000&&text.len()<=24000&&!text.contains('\0'))
            || mentions.len() > 20
        {
            return Err(Error::InvalidData);
        }
        let mut bytes = [0u8; 24];
        OsRng.fill_bytes(&mut bytes);
        let nonce = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let request = SendMessage {
            nonce,
            content,
            reply_to,
            mentions,
            mention_everyone,
        };
        self.inner.cache.enqueue(id, &request).await?;
        Ok(request)
    }
    pub async fn flush(&self) -> Result<usize> {
        let _lock = self.inner.gate.lock().await;
        self.active()?;
        self.checked(self.flush_locked().await).await
    }
    async fn flush_locked(&self) -> Result<usize> {
        // Authoritative current permissions before transmitting queued content.
        self.reconcile_locked().await?;
        let mut sent = 0;
        for pending in self.inner.cache.pending().await? {
            if pending.state == SendState::Failed {
                continue;
            }
            let id = pending.conversation_id;
            match self
                .inner
                .api
                .post::<_, Message>(&format!("/conversations/{id}/messages"), &pending.request)
                .await
            {
                Ok(m) => {
                    if m.author_id != self.inner.user.id || m.nonce != pending.request.nonce {
                        return Err(Error::InvalidData);
                    }
                    self.inner.cache.apply(id, &[m], None).await?;
                    sent += 1;
                }
                Err(e) if e.unauthorized() => return Err(e),
                Err(e) if e.inaccessible() => self.inner.cache.purge(id).await?,
                Err(e) if e.retryable() => return Err(e),
                Err(Error::Http(status, _)) => {
                    self.inner
                        .cache
                        .fail(&pending.request.nonce, status)
                        .await?
                }
                Err(e) => return Err(e),
            }
        }
        Ok(sent)
    }
    pub async fn retry(&self, nonce: &str) -> Result<()> {
        let _lock = self.inner.gate.lock().await;
        self.active()?;
        self.inner.cache.retry(nonce).await
    }
    pub async fn discard(&self, nonce: &str) -> Result<()> {
        let _lock = self.inner.gate.lock().await;
        self.inner.cache.discard(nonce).await
    }
    pub async fn older(&self, id: i64, before: Option<i64>) -> Result<MessagePage> {
        let _lock = self.inner.gate.lock().await;
        self.active()?;
        let suffix = before.map(|v| format!("&before={v}")).unwrap_or_default();
        let result: Result<MessagePage> = self
            .inner
            .api
            .get(&format!("/conversations/{id}/messages?limit=100{suffix}"))
            .await;
        let page = match self.checked(result).await {
            Ok(p) => p,
            Err(e) if e.inaccessible() => {
                self.inner.cache.purge(id).await?;
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        self.inner.cache.apply(id, &page.messages, None).await?;
        Ok(page)
    }
    pub async fn mutate(&self, id: i64, message: i64, command: &MessageCommand) -> Result<Message> {
        let _lock = self.inner.gate.lock().await;
        self.active()?;
        let result = self
            .inner
            .api
            .post(&format!("/conversations/{id}/messages/{message}"), command)
            .await;
        let m = self.checked(result).await?;
        self.inner
            .cache
            .apply(id, std::slice::from_ref(&m), None)
            .await?;
        Ok(m)
    }
}
