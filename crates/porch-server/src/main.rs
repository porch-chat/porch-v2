use porch_server::{Config, MIGRATOR, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();
    let config = Config::from_env()?;
    // Fail startup rather than serving against an unknown schema.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    if std::env::args().nth(1).as_deref() == Some("bootstrap-admin") {
        let user = porch_server::identity::bootstrap(
            &pool,
            std::env::var("PORCH_BOOTSTRAP_USERNAME")?,
            std::env::var("PORCH_BOOTSTRAP_PASSWORD")?,
        )
        .await
        .map_err(|e| format!("bootstrap failed: {:?}", e.0))?;
        println!("created administrator {}", user.username);
        return Ok(());
    }
    let state = porch_server::identity::IdentityState::new(pool.clone())
        .await
        .map_err(|e| format!("identity initialization failed: {:?}", e.0))?;
    porch_server::communities::initialize(&pool)
        .await
        .map_err(|e| format!("community initialization failed: {:?}", e.0))?;
    let gateway = porch_server::gateway::Gateway::new(state.clone());
    let bus: std::sync::Arc<dyn porch_server::gateway::EventBus> =
        std::sync::Arc::new(gateway.clone());
    state.set_event_bus(&bus);
    let communities =
        porch_server::communities::CommunityService::new(pool.clone(), state.clone(), bus.clone());
    let messaging =
        porch_server::messaging::MessagingService::new(pool.clone(), state.clone(), bus);
    let files = match std::env::var_os("PORCH_STORAGE_ROOT") {
        Some(root) => Some(porch_server::files::FileService::new(
            pool.clone(),
            state.clone(),
            std::sync::Arc::new(porch_server::storage::LocalStorage::new(
                std::path::Path::new(&root),
            )?),
        )),
        None => None,
    };
    let mut app = router()
        .merge(porch_server::identity::routes(state.clone()))
        .merge(gateway.routes())
        .merge(communities.routes())
        .merge(messaging.routes());
    if let Some(mut files) = files {
        if let Some(binary) = std::env::var_os("PORCH_FFMPEG_BIN") {
            let limiter = std::env::var_os("PORCH_PRLIMIT_BIN")
                .ok_or("PORCH_PRLIMIT_BIN required for video previews")?;
            files = files.with_video_previews(porch_server::previews::VideoPreviewer::new(
                binary.into(),
                limiter.into(),
            )?);
        }
        app = app.merge(files.routes());
    }
    // Outbound preview traffic is disabled unless the operator explicitly opts in.
    if std::env::var("PORCH_EMBEDS").as_deref() == Ok("1") {
        app = app
            .merge(porch_server::embeds::EmbedService::new(pool.clone(), state.clone()).routes());
    }
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %listener.local_addr()?, "Porch listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "shutdown signal failed");
            }
        })
        .await?;
    pool.close().await;
    Ok(())
}
