//! Entry point: parse configuration, open the database, serve forever.

use anyhow::Context;
use clap::Parser;
use maine_sync_server::api::{self, AppState};
use maine_sync_server::config::{Cli, Config};
use maine_sync_server::db::Db;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from(Cli::parse());
    init_tracing();

    let database = config.database_path();
    let db = Db::open(&database).with_context(|| format!("opening {}", database.display()))?;
    let app = api::router(AppState::new(db, config.clone()));

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    tracing::info!(
        bind = %config.bind,
        data_dir = %config.data_dir.display(),
        keep_versions = config.keep_versions,
        max_blob = config.max_blob,
        creation_protected = config.create_secret.is_some(),
        "maine-sync-server listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving requests")?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            stream.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutting down");
}
