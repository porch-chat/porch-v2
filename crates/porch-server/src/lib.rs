pub mod gateway;
pub mod identity;
use axum::{Json, Router, routing::get};
use porch_protocol::HealthResponse;
use std::{env, net::SocketAddr};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::parse(
            env::var("PORCH_BIND").unwrap_or_else(|_| "127.0.0.1:38147".into()),
            env::var("DATABASE_URL")?,
        )
    }

    pub fn parse(
        bind: String,
        database_url: String,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".into());
        }
        Ok(Self {
            bind: bind.parse()?,
            database_url,
        })
    }
}

pub fn router() -> Router {
    Router::new().route("/health", get(async || Json(HealthResponse::alive())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_contract() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<HealthResponse>(&bytes).unwrap(),
            HealthResponse::alive()
        );
    }

    #[test]
    fn configuration_validation() {
        assert!(
            Config::parse("not-an-address".into(), "postgres://localhost/porch".into()).is_err()
        );
        assert!(Config::parse("127.0.0.1:38147".into(), " ".into()).is_err());
        assert!(
            Config::parse(
                "127.0.0.1:38147".into(),
                "postgres://localhost/porch".into()
            )
            .is_ok()
        );
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres DATABASE_URL; CI runs this explicitly"]
    async fn postgres_migrations() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count as usize,
            MIGRATOR
                .iter()
                .filter(|m| m.migration_type.is_up_migration())
                .count()
        );
        pool.close().await;
    }
}

pub mod permissions;

pub mod communities;

pub mod messaging;

pub mod storage;

pub mod files;
