use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use porch_protocol::*;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct IdentityState {
    pool: PgPool,
    limiter: Arc<Mutex<(Instant, u32)>>,
    hashing: Arc<Semaphore>,
    dummy_hash: Arc<String>,
}

impl IdentityState {
    pub async fn new(pool: PgPool) -> Result<Self, ApiError> {
        Ok(Self {
            pool,
            limiter: Arc::new(Mutex::new((Instant::now(), 0))),
            hashing: Arc::new(Semaphore::new(4)),
            dummy_hash: Arc::new(hash_password(random_secret()).await?),
        })
    }

    fn throttle(&self) -> Result<(), ApiError> {
        // Deliberately instance-wide: cannot be bypassed with spoofed forwarding
        // headers, rotating usernames or source addresses. Bound CPU work and memory.
        let mut window = self
            .limiter
            .lock()
            .map_err(|_| ApiError(ErrorCode::Internal))?;
        if window.0.elapsed() >= Duration::from_secs(60) {
            *window = (Instant::now(), 0);
        }
        if window.1 >= 30 {
            return Err(ApiError(ErrorCode::RateLimited));
        }
        window.1 += 1;
        Ok(())
    }

    async fn password_hash(&self, password: String) -> Result<String, ApiError> {
        let _permit = self
            .hashing
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError(ErrorCode::RateLimited))?;
        hash_password(password).await
    }

    async fn verify(&self, password: String, hash: String) -> Result<bool, ApiError> {
        let _permit = self
            .hashing
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError(ErrorCode::RateLimited))?;
        tokio::task::spawn_blocking(move || {
            let hash = PasswordHash::new(&hash).map_err(|_| ApiError(ErrorCode::Internal))?;
            Ok(Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok())
        })
        .await
        .map_err(|_| ApiError(ErrorCode::Internal))?
    }
}

#[derive(Debug)]
pub struct ApiError(pub ErrorCode);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ErrorCode::InvalidRequest | ErrorCode::InvalidInvite => StatusCode::BAD_REQUEST,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let mut response = (status, Json(ErrorResponse { code: self.0 })).into_response();
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert("retry-after", "60".parse().unwrap());
        }
        response
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        if error
            .as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Self(ErrorCode::Conflict);
        }
        tracing::error!(%error, "identity database failure");
        Self(ErrorCode::Internal)
    }
}

pub fn routes(state: IdentityState) -> Router {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/me", get(me))
        .route("/auth/sessions", get(sessions).delete(logout_all))
        .route("/auth/sessions/{id}", delete(revoke_session))
        .route("/auth/logout", post(logout))
        .route("/invites", post(create_invite).get(list_invites))
        .route("/invites/{id}", delete(revoke_invite))
        .layer(DefaultBodyLimit::max(4096))
        .layer(axum::middleware::map_response(
            async |mut response: Response| {
                response
                    .headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                response
            },
        ))
        .with_state(state)
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digest(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|_| ApiError(ErrorCode::Internal))
    })
    .await
    .map_err(|_| ApiError(ErrorCode::Internal))?
}

fn normalize(username: &str) -> Result<String, ApiError> {
    let name = username.to_ascii_lowercase();
    if !(3..=32).contains(&name.len())
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(ApiError(ErrorCode::InvalidRequest));
    }
    Ok(name)
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if !(12..=128).contains(&password.len()) {
        return Err(ApiError(ErrorCode::InvalidRequest));
    }
    Ok(())
}

fn user(row: &sqlx::postgres::PgRow) -> User {
    User {
        id: row.get("id"),
        username: row.get("username"),
        admin: row.get("admin"),
        can_invite: row.get("can_invite"),
    }
}

async fn new_session(conn: &mut sqlx::PgConnection, user: User) -> Result<AuthResponse, ApiError> {
    let token = random_secret();
    sqlx::query("INSERT INTO sessions(user_id,token_hash) VALUES($1,$2)")
        .bind(user.id)
        .bind(digest(&token))
        .execute(conn)
        .await?;
    Ok(AuthResponse { user, token })
}

async fn register(
    State(state): State<IdentityState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    state.throttle()?;
    let username = normalize(&req.username)?;
    validate_password(&req.password)?;
    if req.invite_code.len() != 64 {
        return Err(ApiError(ErrorCode::InvalidInvite));
    }
    let hash = state.password_hash(req.password).await?;
    let mut tx = state.pool.begin().await?;
    // Atomic guarded consumption serializes concurrent final-use registrations.
    let invite = sqlx::query("UPDATE invites SET uses=uses+1 WHERE code_hash=$1 AND revoked_at IS NULL AND expires_at>now() AND uses<max_uses RETURNING id")
        .bind(digest(&req.invite_code)).fetch_optional(&mut *tx).await?;
    if invite.is_none() {
        return Err(ApiError(ErrorCode::InvalidInvite));
    }
    // Serialize the small platform membership cap with the bootstrap operation.
    sqlx::query("SELECT pg_advisory_xact_lock(7160321)")
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if count >= 200 {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let row = sqlx::query("INSERT INTO users(username) VALUES($1) RETURNING *")
        .bind(&username)
        .fetch_one(&mut *tx)
        .await?;
    let u = user(&row);
    sqlx::query("INSERT INTO identities(user_id,provider,subject,password_hash) VALUES($1,'password',$2,$3)")
        .bind(u.id).bind(username).bind(hash).execute(&mut *tx).await?;
    let response = new_session(&mut tx, u).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn login(
    State(state): State<IdentityState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    state.throttle()?;
    let username = normalize(&req.username)?;
    validate_password(&req.password)?;
    let row = sqlx::query("SELECT u.*, i.password_hash FROM identities i JOIN users u ON u.id=i.user_id WHERE i.provider='password' AND i.subject=$1")
        .bind(username).fetch_optional(&state.pool).await?;
    let hash = row
        .as_ref()
        .map(|r| r.get::<String, _>("password_hash"))
        .unwrap_or_else(|| (*state.dummy_hash).clone());
    let valid = state.verify(req.password, hash).await?;
    let row = row
        .filter(|_| valid)
        .ok_or(ApiError(ErrorCode::Unauthorized))?;
    let mut conn = state.pool.acquire().await?;
    Ok(Json(new_session(&mut conn, user(&row)).await?))
}

async fn authenticate(state: &IdentityState, headers: &HeaderMap) -> Result<(User, i64), ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|s| s.len() == 64)
        .ok_or(ApiError(ErrorCode::Unauthorized))?;
    let row = sqlx::query("SELECT u.*, s.id AS session_id FROM sessions s JOIN users u ON u.id=s.user_id WHERE token_hash=$1 AND revoked_at IS NULL AND expires_at>now()")
        .bind(digest(token)).fetch_optional(&state.pool).await?
        .ok_or(ApiError(ErrorCode::Unauthorized))?;
    Ok((user(&row), row.get("session_id")))
}

async fn me(
    State(state): State<IdentityState>,
    headers: HeaderMap,
) -> Result<Json<User>, ApiError> {
    Ok(Json(authenticate(&state, &headers).await?.0))
}

async fn sessions(
    State(state): State<IdentityState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Session>>, ApiError> {
    let (u, current) = authenticate(&state, &headers).await?;
    let rows = sqlx::query("SELECT id,created_at::text,expires_at::text FROM sessions WHERE user_id=$1 AND revoked_at IS NULL AND expires_at>now() ORDER BY id")
        .bind(u.id).fetch_all(&state.pool).await?;
    Ok(Json(
        rows.iter()
            .map(|r| Session {
                id: r.get("id"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                current: r.get::<i64, _>("id") == current,
            })
            .collect(),
    ))
}

async fn revoke_session(
    State(state): State<IdentityState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let (u, _) = authenticate(&state, &headers).await?;
    let result = sqlx::query(
        "UPDATE sessions SET revoked_at=now() WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(u.id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError(ErrorCode::NotFound));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn logout(
    State(state): State<IdentityState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let (_, id) = authenticate(&state, &headers).await?;
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout_all(
    State(state): State<IdentityState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let (u, _) = authenticate(&state, &headers).await?;
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL")
        .bind(u.id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn invite(row: &sqlx::postgres::PgRow) -> Invite {
    Invite {
        id: row.get("id"),
        creator_id: row.get("creator_id"),
        max_uses: row.get("max_uses"),
        uses: row.get("uses"),
        expires_at: row.get("expiry"),
        revoked: row.get::<Option<String>, _>("revoked").is_some(),
    }
}

async fn create_invite(
    State(state): State<IdentityState>,
    headers: HeaderMap,
    Json(req): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreatedInvite>), ApiError> {
    state.throttle()?;
    let (u, _) = authenticate(&state, &headers).await?;
    if !u.admin && !u.can_invite {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if !(1..=200).contains(&req.max_uses) || !(60..=2_592_000).contains(&req.expires_in_seconds) {
        return Err(ApiError(ErrorCode::InvalidRequest));
    }
    let code = random_secret();
    let row = sqlx::query("INSERT INTO invites(creator_id,code_hash,max_uses,expires_at) VALUES($1,$2,$3,now()+$4 * interval '1 second') RETURNING *, expires_at::text AS expiry, revoked_at::text AS revoked")
        .bind(u.id).bind(digest(&code)).bind(req.max_uses).bind(req.expires_in_seconds as f64).fetch_one(&state.pool).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvite {
            invite: invite(&row),
            code,
        }),
    ))
}

async fn list_invites(
    State(state): State<IdentityState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Invite>>, ApiError> {
    let (u, _) = authenticate(&state, &headers).await?;
    let rows = sqlx::query("SELECT *, expires_at::text AS expiry, revoked_at::text AS revoked FROM invites WHERE creator_id=$1 OR $2 ORDER BY id")
        .bind(u.id).bind(u.admin).fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(invite).collect()))
}

async fn revoke_invite(
    State(state): State<IdentityState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let (u, _) = authenticate(&state, &headers).await?;
    let result = sqlx::query("UPDATE invites SET revoked_at=now() WHERE id=$1 AND (creator_id=$2 OR $3) AND revoked_at IS NULL")
        .bind(id).bind(u.id).bind(u.admin).execute(&state.pool).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError(ErrorCode::NotFound));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// One-time local bootstrap, not an HTTP endpoint. Never upgrades an existing user.
pub async fn bootstrap(
    pool: &PgPool,
    username: String,
    password: String,
) -> Result<User, ApiError> {
    let username = normalize(&username)?;
    validate_password(&password)?;
    let hash = hash_password(password).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7160321)")
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if count != 0 {
        return Err(ApiError(ErrorCode::Conflict));
    }
    let row = sqlx::query(
        "INSERT INTO users(username,admin,can_invite) VALUES($1,true,true) RETURNING *",
    )
    .bind(&username)
    .fetch_one(&mut *tx)
    .await?;
    let u = user(&row);
    sqlx::query("INSERT INTO identities(user_id,provider,subject,password_hash) VALUES($1,'password',$2,$3)")
        .bind(u.id).bind(username).bind(hash).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert_eq!(normalize("AbC_123").unwrap(), "abc_123");
        for value in ["", "ab", "bad name", "éabc", "abc-"] {
            assert!(normalize(value).is_err());
        }
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"a".repeat(129)).is_err());
    }

    #[tokio::test]
    async fn hashing_and_limit() {
        let state = IdentityState::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/test")
                .unwrap(),
        )
        .await
        .unwrap();
        let hash = state
            .password_hash("long password 123".into())
            .await
            .unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(
            state
                .verify("long password 123".into(), hash.clone())
                .await
                .unwrap()
        );
        assert!(
            !state
                .verify("wrong password 123".into(), hash)
                .await
                .unwrap()
        );
        for _ in 0..30 {
            state.throttle().unwrap();
        }
        assert_eq!(state.throttle().unwrap_err().0, ErrorCode::RateLimited);
        *state.limiter.lock().unwrap() = (Instant::now() - Duration::from_secs(61), 30);
        assert!(state.throttle().is_ok());
    }
}
