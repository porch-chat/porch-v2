//! Community persistence and HTTP boundaries. Authorization and save share a row lock.
use crate::{
    gateway::EventBus,
    identity::{ApiError, IdentityState, authenticate, digest, random_secret},
    permissions::Community,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{delete, get, post},
};
use porch_protocol::{permissions::*, *};
use sqlx::{PgConnection, PgPool, Row};
use std::sync::Arc;
type Result<T> = std::result::Result<T, ApiError>;
fn error(code: ErrorCode) -> ApiError {
    ApiError(code)
}

#[derive(Clone)]
pub struct CommunityService {
    pool: PgPool,
    identity: IdentityState,
    bus: Arc<dyn EventBus>,
}
impl CommunityService {
    pub fn new(pool: PgPool, identity: IdentityState, bus: Arc<dyn EventBus>) -> Self {
        Self {
            pool,
            identity,
            bus,
        }
    }
    pub fn routes(self) -> Router {
        Router::new()
            .route("/communities", get(list).post(create))
            .route("/communities/join", post(join))
            .route("/communities/{id}", get(snapshot).post(mutate))
            .route(
                "/communities/{id}/invites",
                get(invites).post(create_invite),
            )
            .route(
                "/communities/{id}/invites/{invite_id}",
                delete(revoke_invite),
            )
            .layer(DefaultBodyLimit::max(65536))
            .layer(axum::middleware::map_response(async |mut r: Response| {
                r.headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                r
            }))
            .with_state(self)
    }
    fn publish(&self, old: &Community, new: Option<&Community>) {
        self.bus
            .publish(&audience(old, new), GatewayEvent::SyncRequired);
    }
}
pub(crate) fn audience(old: &Community, new: Option<&Community>) -> Vec<i64> {
    let mut users: Vec<_> = old
        .members
        .iter()
        .chain(new.into_iter().flat_map(|c| c.members.iter()))
        .map(|m| m.user_id)
        .collect();
    users.sort();
    users.dedup();
    users
}
pub(crate) fn decode(value: serde_json::Value) -> Result<Community> {
    let c: Community = serde_json::from_value(value).map_err(|_| error(ErrorCode::Internal))?;
    c.validate().map_err(|_| error(ErrorCode::Internal))?;
    Ok(c)
}
async fn locked(conn: &mut PgConnection, id: i64) -> Result<Community> {
    let data: serde_json::Value =
        sqlx::query_scalar("SELECT data FROM communities WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(conn)
            .await?
            .ok_or(error(ErrorCode::NotFound))?;
    decode(data)
}
async fn save(conn: &mut PgConnection, c: &Community) -> Result<()> {
    c.validate()?;
    sqlx::query("UPDATE communities SET owner_id=$2,name=$3,data=$4 WHERE id=$1")
        .bind(c.summary.id)
        .bind(c.summary.owner_id)
        .bind(&c.summary.name)
        .bind(serde_json::to_value(c).map_err(|_| error(ErrorCode::Internal))?)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM community_members WHERE community_id=$1")
        .bind(c.summary.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM community_bans WHERE community_id=$1")
        .bind(c.summary.id)
        .execute(&mut *conn)
        .await?;
    for m in &c.members {
        sqlx::query("INSERT INTO community_members(community_id,user_id) VALUES($1,$2)")
            .bind(c.summary.id)
            .bind(m.user_id)
            .execute(&mut *conn)
            .await?;
    }
    for user in &c.bans {
        sqlx::query("INSERT INTO community_bans(community_id,user_id) VALUES($1,$2)")
            .bind(c.summary.id)
            .bind(user)
            .execute(&mut *conn)
            .await?;
    }
    let ids: Vec<i64> = c
        .channels
        .iter()
        .filter(|ch| ch.kind == ChannelKind::Text)
        .map(|ch| ch.id)
        .collect();
    sqlx::query("DELETE FROM conversations WHERE community_id=$1 AND NOT(channel_id=ANY($2))")
        .bind(c.summary.id)
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    for ch in c.channels.iter().filter(|ch| ch.kind == ChannelKind::Text) {
        sqlx::query("INSERT INTO conversations(community_id,channel_id,name) VALUES($1,$2,$3) ON CONFLICT(community_id,channel_id) DO UPDATE SET name=EXCLUDED.name")
            .bind(c.summary.id).bind(ch.id).bind(&ch.name).execute(&mut *conn).await?;
    }
    Ok(())
}
async fn insert(
    conn: &mut PgConnection,
    owner: i64,
    name: String,
    official: bool,
) -> Result<Community> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO communities(owner_id,name,official,data) VALUES($1,$2,$3,'{}') RETURNING id",
    )
    .bind(owner)
    .bind(&name)
    .bind(official)
    .fetch_one(&mut *conn)
    .await?;
    let c = Community::new(id, owner, name, official)?;
    save(conn, &c).await?;
    Ok(c)
}
/// Caller holds platform registration lock when adding a new user. Always lock
/// creation advisory then official community. Bootstrap calls this in its own tx.
pub(crate) async fn seed_official(conn: &mut PgConnection) -> Result<Vec<i64>> {
    sqlx::query("SELECT pg_advisory_xact_lock(7160322)")
        .execute(&mut *conn)
        .await?;
    let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM communities WHERE official")
        .fetch_optional(&mut *conn)
        .await?;
    let mut c = if let Some(id) = existing {
        locked(conn, id).await?
    } else {
        let Some(owner) =
            sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE admin ORDER BY id LIMIT 1")
                .fetch_optional(&mut *conn)
                .await?
        else {
            return Ok(vec![]);
        };
        let mut c = insert(conn, owner, "Porch Announcements".into(), true).await?;
        c.mutate(
            owner,
            CommunityCommand::CreateChannel {
                name: "announcements".into(),
                kind: ChannelKind::Text,
                category_id: None,
                position: 0,
                synced: false,
            },
        )?;
        c
    };
    // Reconcile preexisting users only at initial seed. Later joins must not undo
    // a voluntary leave or a legitimate ban on every startup/registration.
    let users: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE NOT official_initialized ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    let old = c.clone();
    let mut changed = false;
    for user in users {
        if c.member(user).is_none() && !c.bans.contains(&user) {
            c.join(user)?;
            changed = true;
        }
        sqlx::query("UPDATE users SET official_initialized=true WHERE id=$1")
            .bind(user)
            .execute(&mut *conn)
            .await?;
    }
    if changed || existing.is_none() {
        save(conn, &c).await?;
        Ok(audience(&old, Some(&c)))
    } else {
        Ok(vec![])
    }
}
pub async fn initialize(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7160321)")
        .execute(&mut *tx)
        .await?;
    seed_official(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}
async fn list(
    State(s): State<CommunityService>,
    h: HeaderMap,
) -> Result<Json<Vec<CommunitySummary>>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    // Aggregate membership is authoritative. Projection narrows candidate reads;
    // statement MVCC gives one consistent version for the projection and data.
    let rows=sqlx::query_scalar::<_,serde_json::Value>("SELECT c.data FROM communities c JOIN community_members m ON m.community_id=c.id WHERE m.user_id=$1 ORDER BY c.id").bind(u.id).fetch_all(&s.pool).await?;
    let mut result = vec![];
    for row in rows {
        let c = decode(row)?;
        if c.member(u.id).is_some() {
            result.push(c.summary);
        }
    }
    Ok(Json(result))
}
async fn create(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Json(req): Json<CreateCommunity>,
) -> Result<(StatusCode, Json<CommunitySummary>)> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    if !crate::permissions::valid_name(&req.name) {
        return Err(error(ErrorCode::InvalidRequest));
    }
    let mut tx = s.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7160322)")
        .execute(&mut *tx)
        .await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM communities")
        .fetch_one(&mut *tx)
        .await?;
    let owned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM communities WHERE owner_id=$1 AND NOT official")
            .bind(u.id)
            .fetch_one(&mut *tx)
            .await?;
    if total >= 200 || owned >= 10 {
        return Err(error(ErrorCode::Forbidden));
    }
    let c = insert(&mut tx, u.id, req.name, false).await?;
    tx.commit().await?;
    s.publish(&c, None);
    Ok((StatusCode::CREATED, Json(c.summary)))
}
async fn snapshot(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<CommunitySnapshot>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let data = sqlx::query_scalar("SELECT data FROM communities WHERE id=$1")
        .bind(id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(error(ErrorCode::NotFound))?;
    Ok(Json(decode(data)?.snapshot(u.id)?))
}
async fn mutate(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(cmd): Json<CommunityCommand>,
) -> Result<Json<CommunityMutation>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let mut tx = s.pool.begin().await?;
    let old = locked(&mut tx, id).await?;
    let mut c = old.clone();
    let deleted = matches!(cmd, CommunityCommand::Delete);
    // All member IDs must name real users, including bans of nonmembers.
    let target = match &cmd {
        CommunityCommand::Ban { user_id }
        | CommunityCommand::TransferOwnership { user_id }
        | CommunityCommand::SetMemberRoles { user_id, .. } => Some(*user_id),
        _ => None,
    };
    if let Some(target) = target {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1)")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
        if !exists {
            return Err(error(ErrorCode::InvalidRequest));
        }
    }
    let output = c.mutate(u.id, cmd)?;
    if deleted {
        sqlx::query("DELETE FROM communities WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        save(&mut tx, &c).await?;
    }
    tx.commit().await?;
    s.publish(&old, if deleted { None } else { Some(&c) });
    Ok(Json(CommunityMutation { id: output }))
}
fn invite_access(c: &Community, user: i64, permission: u64) -> Result<()> {
    // Obscure existence to outsiders; platform administrators do not bypass
    // community authorization.
    if c.member(user).is_none() {
        return Err(error(ErrorCode::NotFound));
    }
    if c.base(user) & permission != permission {
        return Err(error(ErrorCode::Forbidden));
    }
    Ok(())
}
fn invite_row(row: &sqlx::postgres::PgRow) -> CommunityInvite {
    CommunityInvite {
        id: row.get("id"),
        creator_id: row.get("creator_id"),
        max_uses: row.get("max_uses"),
        uses: row.get("uses"),
        expires_at: row.get("expiry"),
        revoked: row.get("revoked"),
    }
}
async fn create_invite(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreatedCommunityInvite>)> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    if !(1..=200).contains(&req.max_uses) || !(60..=2_592_000).contains(&req.expires_in_seconds) {
        return Err(error(ErrorCode::InvalidRequest));
    }
    let mut tx = s.pool.begin().await?;
    let c = locked(&mut tx, id).await?;
    invite_access(&c, u.id, CREATE_INVITE)?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM community_invites WHERE community_id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 500 {
        return Err(error(ErrorCode::Forbidden));
    }
    let code = random_secret();
    let invite_id:i64=sqlx::query_scalar("INSERT INTO community_invites(community_id,creator_id,code_hash,max_uses,expires_at) VALUES($1,$2,$3,$4,now()+$5 * interval '1 second') RETURNING id")
        .bind(id).bind(u.id).bind(digest(&code)).bind(req.max_uses).bind(req.expires_in_seconds as f64).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    s.publish(&c, None);
    Ok((
        StatusCode::CREATED,
        Json(CreatedCommunityInvite {
            id: invite_id,
            community_id: id,
            code,
        }),
    ))
}
async fn invites(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Vec<CommunityInvite>>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let mut tx = s.pool.begin().await?;
    let c = locked(&mut tx, id).await?;
    invite_access(&c, u.id, CREATE_INVITE)?;
    let manager = c.base(u.id) & MANAGE_COMMUNITY != 0;
    let rows=sqlx::query("SELECT *,expires_at::text AS expiry,revoked_at IS NOT NULL AS revoked FROM community_invites WHERE community_id=$1 AND (creator_id=$2 OR $3) ORDER BY id")
        .bind(id).bind(u.id).bind(manager).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(rows.iter().map(invite_row).collect()))
}
async fn revoke_invite(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Path((id, invite_id)): Path<(i64, i64)>,
) -> Result<StatusCode> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    let mut tx = s.pool.begin().await?;
    let c = locked(&mut tx, id).await?;
    invite_access(&c, u.id, CREATE_INVITE)?;
    let result=sqlx::query("UPDATE community_invites SET revoked_at=now() WHERE id=$1 AND community_id=$2 AND (creator_id=$3 OR $4) AND revoked_at IS NULL")
        .bind(invite_id).bind(id).bind(u.id).bind(c.base(u.id)&MANAGE_COMMUNITY!=0).execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Err(error(ErrorCode::NotFound));
    }
    tx.commit().await?;
    s.publish(&c, None);
    Ok(StatusCode::NO_CONTENT)
}
async fn join(
    State(s): State<CommunityService>,
    h: HeaderMap,
    Json(req): Json<JoinCommunity>,
) -> Result<Json<CommunitySummary>> {
    let (u, _) = authenticate(&s.identity, &h).await?;
    if req.code.len() != 64 {
        return Err(error(ErrorCode::InvalidInvite));
    }
    let hash = digest(&req.code);
    let mut tx = s.pool.begin().await?;
    // Lookup doesn't lock an invite before community, avoiding lock inversion.
    let id: i64 =
        sqlx::query_scalar("SELECT community_id FROM community_invites WHERE code_hash=$1")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(error(ErrorCode::InvalidInvite))?;
    let old = locked(&mut tx, id).await?;
    let mut c = old.clone();
    c.join(u.id)?;
    let consumed=sqlx::query("UPDATE community_invites SET uses=uses+1 WHERE code_hash=$1 AND community_id=$2 AND revoked_at IS NULL AND expires_at>now() AND uses<max_uses")
        .bind(hash).bind(id).execute(&mut *tx).await?;
    if consumed.rows_affected() != 1 {
        return Err(error(ErrorCode::InvalidInvite));
    }
    save(&mut tx, &c).await?;
    tx.commit().await?;
    s.publish(&old, Some(&c));
    Ok(Json(c.summary))
}
