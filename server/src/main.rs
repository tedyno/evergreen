//! homesign server — self-hosted sideloading pro iPad/iPhone.
//! Viz docs/architecture.md.

mod apple;
mod config;
mod crypto;
mod db;
mod device;
mod error;
mod ipa;
mod jobs;
mod models;
mod pipeline;
mod refresh;
mod signer;
mod state;
mod web;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "homesign_server=info,tower_http=warn".into()),
        )
        .init();

    let cfg = Config::from_env()?;
    tracing::info!("data dir: {}", cfg.data_dir.display());
    tracing::info!("anisette: {}", cfg.anisette_url);

    let pool = db::connect(&cfg.db_url()).await?;
    let state = AppState::new(cfg.clone(), pool);

    // Refresh scheduler (jádro — obnova 7denních profilů ze serveru).
    refresh::spawn(state.clone());

    let app = web::router(state);
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!("homesign poslouchá na http://{}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
