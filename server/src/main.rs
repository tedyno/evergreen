//! homesign server — self-hosted sideloading pro iPad/iPhone.
//! Viz docs/architecture.md.

mod apple;
mod codesign;
mod config;
mod crypto;
mod db;
mod device;
mod error;
mod ipa;
mod jobs;
mod models;
mod pairing;
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

    // Úklid: úlohy, co „visely" při minulém běhu (server se restartoval během nich).
    let _ = sqlx::query(
        "UPDATE job SET status = 'error', message = 'přerušeno (restart serveru)', updated_at = ?
         WHERE status IN ('running', 'queued')",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&pool)
    .await;

    let state = AppState::new(cfg.clone(), pool);

    // Obnov přihlášenou Apple ID session z disku, ať přežije restart serveru.
    if let Err(e) = restore_account_session(&state).await {
        tracing::warn!("Apple ID session se nepodařilo obnovit: {e:?}");
    }

    // Refresh scheduler (jádro — obnova 7denních profilů ze serveru).
    refresh::spawn(state.clone());

    let app = web::router(state);
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!("homesign poslouchá na http://{}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Načte uloženou Apple ID session z DB a obnoví přihlášení (bez hesla/2FA).
async fn restore_account_session(state: &AppState) -> anyhow::Result<()> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT apple_id, session_enc FROM account WHERE id = 1")
            .fetch_optional(&state.db)
            .await?;
    if let Some((apple_id, Some(enc))) = row {
        let xml = crypto::decrypt(&state.cfg.master_key, &enc)?;
        state.apple.restore_session(apple_id, &xml).await?;
        tracing::info!("Apple ID session obnovena z disku");
    }
    Ok(())
}
