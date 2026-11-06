//! Transactional messaging. No content is published into retained gateway replay.
use crate::{
    communities::decode,
    gateway::EventBus,
    identity::{ApiError, IdentityState, authenticate, digest},
    permissions::Community,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
};
use porch_protocol::{permissions::*, *};
use serde::Deserialize;
use sqlx::{PgConnection, PgPool, Row};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, ApiError>;
fn err(code: ErrorCode) -> ApiError {
    ApiError(code)
}
fn invalid() -> ApiError {
    err(ErrorCode::InvalidRequest)
}
fn missing() -> ApiError {
    err(ErrorCode::NotFound)
}
#[derive(Clone)]
pub struct MessagingService {
    pool: PgPool,
    identity: IdentityState,
    bus: Arc<dyn EventBus>,
    rate: Arc<Mutex<HashMap<i64, (Instant, u32)>>>,
}
impl MessagingService {
    pub fn new(pool: PgPool, identity: IdentityState, bus: Arc<dyn EventBus>) -> Self {
        Self {
            pool,
            identity,
            bus,
            rate: Arc::default(),
        }
    }
    pub fn routes(self) -> Router {
        Router::new()
            .route("/conversations", get(heads).post(create_dm))
            .route("/conversations/{id}", post(dm_command))
            .route("/conversations/{id}/messages", get(history).post(send))
            .route("/conversations/{id}/messages/{message}", post(change))
            .route("/conversations/{id}/delta", get(delta))
            .route("/conversations/{id}/search", get(search))
            .route("/conversations/{id}/pins", get(pins))
            .route("/conversations/{id}/read", post(mark_read))
            .layer(DefaultBodyLimit::max(32768))
            .layer(axum::middleware::map_response(async |mut r: Response| {
                r.headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                r
            }))
            .with_state(self)
    }
    fn throttle(&self, user: i64) -> Result<()> {
        throttle(&self.rate, user)
    }

    fn publish(&self, users: &[i64]) {
        self.bus.publish(users, GatewayEvent::SyncRequired);
    }
}
fn throttle(rate: &Mutex<HashMap<i64, (Instant, u32)>>, user: i64) -> Result<()> {
    let mut map = rate.lock().unwrap();
    map.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(60));
    if map.len() >= 200 && !map.contains_key(&user) {
        return Err(err(ErrorCode::RateLimited));
    }
    let entry = map.entry(user).or_insert((Instant::now(), 0));
    if entry.1 >= 120 {
        return Err(err(ErrorCode::RateLimited));
    }
    entry.1 += 1;
    Ok(())
}

pub(crate) struct Context {
    id: i64,
    community: Option<Community>,
    channel: Option<i64>,
    name: String,
    owner: Option<i64>,
    participants: Vec<i64>,
    pair: Option<String>,
    revision: i64,
    bits: u64,
}
impl Context {
    pub(crate) fn require(&self, bits: u64) -> Result<()> {
        if self.bits & bits == bits {
            Ok(())
        } else {
            Err(err(ErrorCode::Forbidden))
        }
    }
    fn audience(&self) -> Vec<i64> {
        match &self.community {
            Some(c) => c
                .channels
                .iter()
                .find(|ch| Some(ch.id) == self.channel)
                .map(|ch| {
                    c.members
                        .iter()
                        .filter(|m| {
                            c.channel_bits(m.user_id, ch) & (VIEW_CHANNEL | READ_HISTORY)
                                == (VIEW_CHANNEL | READ_HISTORY)
                        })
                        .map(|m| m.user_id)
                        .collect()
                })
                .unwrap_or_default(),
            None => self.participants.clone(),
        }
    }
}
pub(crate) async fn context(
    conn: &mut PgConnection,
    id: i64,
    user: i64,
    write: bool,
) -> Result<Context> {
    // Unlocked lookup only chooses the lock order. Recheck the row after locks.
    let cid: Option<i64> = sqlx::query_scalar("SELECT community_id FROM conversations WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(missing)?;
    let community = if let Some(cid) = cid {
        let q = if write {
            "SELECT data FROM communities WHERE id=$1 FOR SHARE"
        } else {
            "SELECT data FROM communities WHERE id=$1"
        };
        let value = sqlx::query_scalar(q)
            .bind(cid)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(missing)?;
        Some(decode(value)?)
    } else {
        None
    };
    let q = if write {
        "SELECT * FROM conversations WHERE id=$1 FOR UPDATE"
    } else {
        "SELECT * FROM conversations WHERE id=$1"
    };
    let row = sqlx::query(q)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(missing)?;
    let channel: Option<i64> = row.get("channel_id");
    let participants: Vec<i64> = row.get("participants");
    let owner: Option<i64> = row.get("owner_id");
    let pair: Option<String> = row.get("dm_pair");
    if row.get::<Option<i64>, _>("community_id") != cid {
        return Err(missing());
    }
    let bits = if let Some(c) = &community {
        let ch = c
            .channels
            .iter()
            .find(|ch| Some(ch.id) == channel && ch.kind == ChannelKind::Text)
            .ok_or_else(missing)?;
        c.channel_bits(user, ch)
    } else if participants.contains(&user) {
        DEFAULT
            | if owner == Some(user) && pair.is_none() {
                MANAGE_MESSAGES
            } else {
                0
            }
    } else {
        0
    };
    if bits & VIEW_CHANNEL == 0 {
        return Err(missing());
    }
    Ok(Context {
        id,
        community,
        channel,
        name: row.get("name"),
        owner,
        participants,
        pair,
        revision: row.get("revision"),
        bits,
    })
}
async fn read_tx(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
async fn bump(conn: &mut PgConnection, id: i64) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "UPDATE conversations SET revision=revision+1 WHERE id=$1 RETURNING revision",
    )
    .bind(id)
    .fetch_one(conn)
    .await?)
}
fn parse_message(value: serde_json::Value) -> Result<Message> {
    serde_json::from_value(value).map_err(|_| err(ErrorCode::Internal))
}
async fn message(conn: &mut PgConnection, id: i64, mid: i64) -> Result<Message> {
    parse_message(
        sqlx::query_scalar("SELECT data FROM messages WHERE conversation_id=$1 AND id=$2")
            .bind(id)
            .bind(mid)
            .fetch_optional(conn)
            .await?
            .ok_or_else(missing)?,
    )
}
async fn persist(conn: &mut PgConnection, m: &Message) -> Result<()> {
    let data = serde_json::to_value(m).map_err(|_| invalid())?;
    if data.to_string().len() > 60000 {
        return Err(invalid());
    }
    let source = match &m.content {
        Some(MessageContent::Markdown { text }) => text.as_str(),
        _ => "",
    };
    sqlx::query("UPDATE messages SET data=$3,source=$4,deleted=$5,mentions=$6,mention_everyone=$7,pinned=$8,revision=$9 WHERE conversation_id=$1 AND id=$2")
        .bind(m.conversation_id).bind(m.id).bind(serde_json::to_value(m).map_err(|_|err(ErrorCode::Internal))?).bind(source).bind(m.deleted).bind(&m.mentions).bind(m.mention_everyone).bind(m.pinned).bind(m.revision).execute(conn).await?;
    Ok(())
}
fn message_html(value: &MessageContent, has_attachments: bool) -> Result<String> {
    if has_attachments && matches!(value, MessageContent::Markdown{text} if text.is_empty()) {
        Ok(String::new())
    } else {
        content(value)
    }
}
fn content(content: &MessageContent) -> Result<String> {
    let MessageContent::Markdown { text } = content else {
        return Err(invalid());
    };
    if text.trim().is_empty() || text.len() > 8000 || text.contains('\0') {
        return Err(invalid());
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(
        &mut html,
        pulldown_cmark::Parser::new_ext(
            text,
            pulldown_cmark::Options::ENABLE_STRIKETHROUGH | pulldown_cmark::Options::ENABLE_TABLES,
        ),
    );
    Ok(ammonia::Builder::default()
        .rm_tags(&["img"])
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .clean(&html)
        .to_string())
}
fn validate_nonce(nonce: &str) -> Result<()> {
    if !(16..=64).contains(&nonce.len())
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid());
    }
    Ok(())
}
fn mentions(ctx: &Context, users: &[i64], everyone: bool) -> Result<()> {
    if users.len() > 20 || users.iter().collect::<HashSet<_>>().len() != users.len() {
        return Err(invalid());
    }
    if everyone {
        ctx.require(MENTION_EVERYONE)?;
    }
    for user in users {
        let valid = if let Some(c) = &ctx.community {
            c.channels
                .iter()
                .find(|ch| Some(ch.id) == ctx.channel)
                .is_some_and(|ch| c.channel_bits(*user, ch) & VIEW_CHANNEL != 0)
        } else {
            ctx.participants.contains(user)
        };
        if !valid {
            return Err(invalid());
        }
    }
    Ok(())
}
async fn head(conn: &mut PgConnection, c: &Context, user: i64) -> Result<ConversationHead> {
    let read: i64 = sqlx::query_scalar(
        "SELECT message_id FROM message_reads WHERE conversation_id=$1 AND user_id=$2",
    )
    .bind(c.id)
    .bind(user)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(0);
    let row=sqlx::query("SELECT max(id) AS last,count(*) FILTER(WHERE id>$2 AND author_id<>$3) AS unread,count(*) FILTER(WHERE id>$2 AND author_id<>$3 AND ($3=ANY(mentions) OR mention_everyone)) AS mentions FROM messages WHERE conversation_id=$1 AND NOT deleted")
        .bind(c.id).bind(read).bind(user).fetch_one(conn).await?;
    Ok(ConversationHead {
        id: c.id,
        community_id: c.community.as_ref().map(|c| c.summary.id),
        channel_id: c.channel,
        name: c.name.clone(),
        owner_id: c.owner,
        participants: c.participants.clone(),
        revision: c.revision,
        last_message_id: row.get("last"),
        read_message_id: read,
        unread_count: row.get("unread"),
        mention_count: row.get("mentions"),
    })
}
async fn heads(
    State(s): State<MessagingService>,
    h: HeaderMap,
) -> Result<Json<Vec<ConversationHead>>> {
    let (user, _) = authenticate(&s.identity, &h).await?;
    let mut tx = read_tx(&s.pool).await?;
    let ids:Vec<i64>=sqlx::query_scalar("SELECT id FROM conversations WHERE $1=ANY(participants) OR community_id IN (SELECT community_id FROM community_members WHERE user_id=$1) ORDER BY id").bind(user.id).fetch_all(&mut *tx).await?;
    let mut out = vec![];
    for id in ids {
        match context(&mut tx, id, user.id, false).await {
            Ok(c) if c.bits & READ_HISTORY != 0 => out.push(head(&mut tx, &c, user.id).await?),
            Ok(_) => {}
            Err(e) if e.0 == ErrorCode::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    tx.commit().await?;
    Ok(Json(out))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    before: Option<i64>,
    after: Option<i64>,
    through: Option<i64>,
    limit: Option<i64>,
    q: Option<String>,
}
impl PageQuery {
    fn limit(&self) -> Result<i64> {
        let n = self.limit.unwrap_or(50);
        if !(1..=100).contains(&n) {
            Err(invalid())
        } else {
            Ok(n)
        }
    }
}
#[derive(Clone, Copy)]
enum PageMode {
    History,
    Delta,
    Search,
    Pins,
}
async fn page(
    s: MessagingService,
    h: HeaderMap,
    id: i64,
    q: PageQuery,
    mode: PageMode,
) -> Result<Json<MessagePage>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let limit = q.limit()?;
    let mut tx = read_tx(&s.pool).await?;
    let ctx = context(&mut tx, id, u.id, false).await?;
    ctx.require(READ_HISTORY)?;
    let high = q.through.unwrap_or(ctx.revision);
    if high < 0
        || high > ctx.revision
        || q.before.is_some_and(|n| n <= 0)
        || q.after.is_some_and(|n| n < 0 || n > high)
    {
        return Err(invalid());
    }
    let mut messages:Vec<Message>=match mode {
        PageMode::History=>sqlx::query_scalar::<_,serde_json::Value>("SELECT data FROM messages WHERE conversation_id=$1 AND id<$2 ORDER BY id DESC LIMIT $3").bind(id).bind(q.before.unwrap_or(i64::MAX)).bind(limit+1).fetch_all(&mut *tx).await?,
        PageMode::Delta=>sqlx::query_scalar("SELECT data FROM messages WHERE conversation_id=$1 AND revision>$2 AND revision<=$3 ORDER BY revision LIMIT $4").bind(id).bind(q.after.unwrap_or(0)).bind(high).bind(limit+1).fetch_all(&mut *tx).await?,
        PageMode::Pins=>sqlx::query_scalar("SELECT data FROM messages WHERE conversation_id=$1 AND NOT deleted AND pinned AND id<$2 ORDER BY id DESC LIMIT $3").bind(id).bind(q.before.unwrap_or(i64::MAX)).bind(limit+1).fetch_all(&mut *tx).await?,
        PageMode::Search=>{let text=q.q.ok_or_else(invalid)?;if text.trim().is_empty()||text.len()>200||text.contains('\0'){return Err(invalid());}sqlx::query_scalar("SELECT data FROM messages WHERE conversation_id=$1 AND NOT deleted AND search @@ plainto_tsquery('simple',$2) AND id<$3 ORDER BY id DESC LIMIT $4").bind(id).bind(text).bind(q.before.unwrap_or(i64::MAX)).bind(limit+1).fetch_all(&mut *tx).await?}
    }.into_iter().map(parse_message).collect::<Result<_>>()?;
    let has_more = messages.len() > limit as usize;
    messages.truncate(limit as usize);
    let next = if has_more {
        messages.last().map(|m| {
            if matches!(mode, PageMode::Delta) {
                m.revision
            } else {
                m.id
            }
        })
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(MessagePage {
        messages,
        next,
        high_watermark: high,
    }))
}
async fn history(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Query(q): Query<PageQuery>,
) -> Result<Json<MessagePage>> {
    page(s, h, id, q, PageMode::History).await
}
async fn delta(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Query(q): Query<PageQuery>,
) -> Result<Json<MessagePage>> {
    page(s, h, id, q, PageMode::Delta).await
}
async fn search(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Query(q): Query<PageQuery>,
) -> Result<Json<MessagePage>> {
    page(s, h, id, q, PageMode::Search).await
}
async fn pins(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Query(q): Query<PageQuery>,
) -> Result<Json<MessagePage>> {
    page(s, h, id, q, PageMode::Pins).await
}
async fn send(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<SendMessage>,
) -> Result<Json<Message>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    s.throttle(u.id)?;
    validate_nonce(&body.nonce)?;
    let html = message_html(&body.content, !body.attachments.is_empty())?;
    let fingerprint = digest(&serde_json::to_string(&body).map_err(|_| invalid())?);
    let mut tx = s.pool.begin().await?;
    let ctx = context(&mut tx, id, u.id, true).await?;
    ctx.require(SEND_MESSAGES | READ_HISTORY)?;
    let prior:Option<(serde_json::Value,String)>=sqlx::query_as("SELECT data,fingerprint FROM messages WHERE conversation_id=$1 AND author_id=$2 AND nonce=$3").bind(id).bind(u.id).bind(&body.nonce).fetch_optional(&mut *tx).await?;
    if let Some((data, hash)) = prior {
        if hash != fingerprint {
            return Err(err(ErrorCode::Conflict));
        }
        let m = parse_message(data)?;
        tx.commit().await?;
        return Ok(Json(m));
    }
    mentions(&ctx, &body.mentions, body.mention_everyone)?;
    if let Some(reply) = body.reply_to {
        message(&mut tx, id, reply).await?;
    }
    let revision = bump(&mut tx, id).await?;
    let row=sqlx::query("INSERT INTO messages(conversation_id,author_id,nonce,fingerprint,reply_to,data,revision) VALUES($1,$2,$3,$4,$5,'{}',$6) RETURNING id,created_at::text")
        .bind(id).bind(u.id).bind(&body.nonce).bind(fingerprint).bind(body.reply_to).bind(revision).fetch_one(&mut *tx).await?;
    let attachments =
        crate::files::bind_attachments(&mut tx, &body.attachments, u.id, id, row.get("id")).await?;
    let m = Message {
        attachments,
        id: row.get("id"),
        conversation_id: id,
        author_id: u.id,
        nonce: body.nonce,
        content: Some(body.content),
        rendered_html: Some(html),
        reply_to: body.reply_to,
        mentions: body.mentions,
        mention_everyone: body.mention_everyone,
        created_at: row.get("created_at"),
        edited_at: None,
        deleted: false,
        pinned: false,
        reactions: vec![],
        revision,
    };
    persist(&mut tx, &m).await?;
    tx.commit().await?;
    s.publish(&ctx.audience());
    Ok(Json(m))
}
async fn change(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path((id, mid)): Path<(i64, i64)>,
    Json(cmd): Json<MessageCommand>,
) -> Result<Json<Message>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    s.throttle(u.id)?;
    let mut tx = s.pool.begin().await?;
    let ctx = context(&mut tx, id, u.id, true).await?;
    ctx.require(READ_HISTORY)?;
    let mut m = message(&mut tx, id, mid).await?;
    if m.deleted {
        if matches!(cmd, MessageCommand::Delete)
            && (m.author_id == u.id || ctx.bits & MANAGE_MESSAGES != 0)
        {
            tx.commit().await?;
            return Ok(Json(m));
        }
        return Err(missing());
    }
    match cmd {
        MessageCommand::Edit {
            content: body,
            mentions: users,
            mention_everyone,
        } => {
            if m.author_id != u.id {
                return Err(err(ErrorCode::Forbidden));
            }
            ctx.require(SEND_MESSAGES)?;
            mentions(&ctx, &users, mention_everyone)?;
            m.rendered_html = Some(message_html(&body, !m.attachments.is_empty())?);
            m.content = Some(body);
            m.mentions = users;
            m.mention_everyone = mention_everyone;
            m.edited_at = Some(
                sqlx::query_scalar("SELECT now()::text")
                    .fetch_one(&mut *tx)
                    .await?,
            );
        }
        MessageCommand::Delete => {
            if m.author_id != u.id {
                ctx.require(MANAGE_MESSAGES)?;
            }
            m.attachments.clear();
            m.content = None;
            m.rendered_html = None;
            m.mentions.clear();
            m.mention_everyone = false;
            m.reactions.clear();
            m.pinned = false;
            m.deleted = true;
        }
        MessageCommand::Pin { pinned } => {
            ctx.require(MANAGE_MESSAGES)?;
            if pinned && !m.pinned {
                let count:i64=sqlx::query_scalar("SELECT count(*) FROM messages WHERE conversation_id=$1 AND pinned AND NOT deleted").bind(id).fetch_one(&mut *tx).await?;
                if count >= 50 {
                    return Err(invalid());
                }
            }
            m.pinned = pinned;
        }
        MessageCommand::React { emoji, active } => {
            ctx.require(SEND_MESSAGES)?;
            if emoji.trim().is_empty() || emoji.len() > 64 || emoji.chars().any(char::is_control) {
                return Err(invalid());
            }
            if let Some(r) = m.reactions.iter_mut().find(|r| r.emoji == emoji) {
                r.users.retain(|id| *id != u.id);
                if active {
                    r.users.push(u.id);
                    r.users.sort();
                }
            } else if active {
                if m.reactions.len() >= 20 {
                    return Err(invalid());
                }
                m.reactions.push(Reaction {
                    emoji,
                    users: vec![u.id],
                });
            }
            m.reactions.retain(|r| !r.users.is_empty());
            m.reactions.sort_by(|a, b| a.emoji.cmp(&b.emoji));
        }
    }
    m.revision = bump(&mut tx, id).await?;
    persist(&mut tx, &m).await?;
    tx.commit().await?;
    s.publish(&ctx.audience());
    Ok(Json(m))
}
async fn mark_read(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<MarkRead>,
) -> Result<StatusCode> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    s.throttle(u.id)?;
    let mut tx = s.pool.begin().await?;
    let ctx = context(&mut tx, id, u.id, true).await?;
    ctx.require(READ_HISTORY)?;
    message(&mut tx, id, body.message_id).await?;
    sqlx::query("INSERT INTO message_reads(conversation_id,user_id,message_id) VALUES($1,$2,$3) ON CONFLICT(conversation_id,user_id) DO UPDATE SET message_id=GREATEST(message_reads.message_id,EXCLUDED.message_id)").bind(id).bind(u.id).bind(body.message_id).execute(&mut *tx).await?;
    tx.commit().await?;
    s.publish(&[u.id]);
    Ok(StatusCode::NO_CONTENT)
}
async fn valid_users(conn: &mut PgConnection, users: &[i64]) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id=ANY($1)")
        .bind(users)
        .fetch_one(conn)
        .await?;
    if count as usize != users.len() {
        return Err(invalid());
    }
    Ok(())
}
async fn create_dm(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Json(body): Json<CreateDm>,
) -> Result<Json<ConversationHead>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    s.throttle(u.id)?;
    if body.participants.is_empty()
        || body.participants.len() > 19
        || body.participants.contains(&u.id)
        || body.participants.iter().collect::<HashSet<_>>().len() != body.participants.len()
    {
        return Err(invalid());
    }
    let mut users = body.participants;
    users.push(u.id);
    users.sort();
    let pair = if users.len() == 2 {
        Some(format!("{}:{}", users[0], users[1]))
    } else {
        None
    };
    let name = body.name.unwrap_or_else(|| {
        if pair.is_some() {
            "Direct message".into()
        } else {
            "Group message".into()
        }
    });
    if !crate::permissions::valid_name(&name) {
        return Err(invalid());
    }
    let mut tx = s.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7160323)")
        .execute(&mut *tx)
        .await?;
    valid_users(&mut tx, &users).await?;
    let existing: Option<i64> = if let Some(pair) = &pair {
        sqlx::query_scalar("SELECT id FROM conversations WHERE dm_pair=$1")
            .bind(pair)
            .fetch_optional(&mut *tx)
            .await?
    } else {
        None
    };
    let id = if let Some(id) = existing {
        id
    } else {
        // Bound membership count for every affected user, not just the creator.
        for user in &users {
            let n: i64 =
                sqlx::query_scalar("SELECT count(*) FROM conversations WHERE $1=ANY(participants)")
                    .bind(user)
                    .fetch_one(&mut *tx)
                    .await?;
            if n >= 100 {
                return Err(invalid());
            }
        }
        sqlx::query_scalar("INSERT INTO conversations(name,owner_id,participants,dm_pair,revision) VALUES($1,$2,$3,$4,1) RETURNING id").bind(name).bind(u.id).bind(&users).bind(pair).fetch_one(&mut *tx).await?
    };
    let ctx = context(&mut tx, id, u.id, true).await?;
    let out = head(&mut tx, &ctx, u.id).await?;
    tx.commit().await?;
    s.publish(&users);
    Ok(Json(out))
}
async fn dm_command(
    State(s): State<MessagingService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(cmd): Json<DmCommand>,
) -> Result<StatusCode> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    s.throttle(u.id)?;
    let mut tx = s.pool.begin().await?;
    // Same global cap lock as create, always before conversation locks.
    sqlx::query("SELECT pg_advisory_xact_lock(7160323)")
        .execute(&mut *tx)
        .await?;
    let ctx = context(&mut tx, id, u.id, true).await?;
    if ctx.community.is_some() || ctx.pair.is_some() {
        return Err(invalid());
    }
    let mut users = ctx.participants.clone();
    let mut owner = ctx.owner.unwrap();
    let mut name = ctx.name.clone();
    if !matches!(cmd, DmCommand::Leave) && owner != u.id {
        return Err(err(ErrorCode::Forbidden));
    }
    match cmd {
        DmCommand::Rename { name: new } => {
            if !crate::permissions::valid_name(&new) {
                return Err(invalid());
            }
            name = new;
        }
        DmCommand::Add { user_id } => {
            if users.len() >= 20 || users.contains(&user_id) {
                return Err(invalid());
            }
            valid_users(&mut tx, &[user_id]).await?;
            let n: i64 =
                sqlx::query_scalar("SELECT count(*) FROM conversations WHERE $1=ANY(participants)")
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if n >= 100 {
                return Err(invalid());
            }
            users.push(user_id);
            users.sort();
        }
        DmCommand::Remove { user_id } => {
            if user_id == owner || !users.contains(&user_id) {
                return Err(invalid());
            }
            users.retain(|id| *id != user_id);
        }
        DmCommand::TransferOwner { user_id } => {
            if !users.contains(&user_id) {
                return Err(invalid());
            }
            owner = user_id;
        }
        DmCommand::Leave => {
            users.retain(|id| *id != u.id);
            if owner == u.id {
                owner = users.first().copied().unwrap_or(0);
            }
        }
    }
    if users.is_empty() {
        sqlx::query("DELETE FROM conversations WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE conversations SET name=$2,owner_id=$3,participants=$4,revision=revision+1 WHERE id=$1").bind(id).bind(name).bind(owner).bind(&users).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM message_reads WHERE conversation_id=$1 AND NOT(user_id=ANY($2))")
            .bind(id)
            .bind(&users)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    users.extend(ctx.participants);
    users.sort();
    users.dedup();
    s.publish(&users);
    Ok(StatusCode::NO_CONTENT)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_is_sanitized() {
        let html=content(&MessageContent::Markdown{text:"**hi** <script>alert(1)</script> ![x](https://example.com/tracker) [bad](javascript:alert(1)) <a href='/auth/logout'>x</a>".into()}).unwrap();
        assert!(html.contains("<strong>hi</strong>"));
        for bad in ["<script", "<img", "javascript:", "href=\"/"] {
            assert!(!html.contains(bad), "{html}");
        }
    }
    #[test]
    fn content_and_nonce_bounds() {
        assert!(content(&MessageContent::Markdown { text: " ".into() }).is_err());
        assert!(
            content(&MessageContent::Ciphertext {
                algorithm: "fake".into(),
                payload: "opaque".into()
            })
            .is_err()
        );
        assert!(validate_nonce("0123456789abcdef").is_ok());
        assert!(validate_nonce("short").is_err());
        assert!(validate_nonce("0123456789abcdef\n").is_err());
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(256))]
        #[test]
        fn untrusted_markdown_stays_inert(text in "[^\\x00]{1,512}") {
            if let Ok(html)=content(&MessageContent::Markdown{text}) {
                let lower=html.to_lowercase();
                for forbidden in ["<script","<iframe","<img","<object","<svg","<style"] {
                    proptest::prop_assert!(!lower.contains(forbidden));
                }
            }
        }
    }
    #[test]
    fn mutation_limit() {
        let rate = Mutex::new(HashMap::new());
        for _ in 0..120 {
            assert!(throttle(&rate, 1).is_ok());
        }
        assert_eq!(throttle(&rate, 1).unwrap_err().0, ErrorCode::RateLimited);
        assert!(throttle(&rate, 2).is_ok());
        rate.lock().unwrap().get_mut(&1).unwrap().0 = Instant::now() - Duration::from_secs(61);
        assert!(throttle(&rate, 1).is_ok());
        for user in 3..=200 {
            assert!(throttle(&rate, user).is_ok());
        }
        assert!(throttle(&rate, 201).is_err());
    }
}
