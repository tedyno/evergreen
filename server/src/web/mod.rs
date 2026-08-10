//! HTTP router: web UI + REST API + statické soubory (ikony IPA).

mod api;
mod ui;

use axum::routing::{get, post};
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        // Web UI
        .route("/", get(ui::index))
        .route("/icon/:id", get(api::icon))
        // --- REST API ---
        .route("/api/status", get(api::status))
        // účet
        .route("/api/account", get(api::account_status))
        .route("/api/account/login", post(api::login))
        .route("/api/account/2fa", post(api::submit_2fa))
        .route("/api/account/logout", post(api::logout))
        // zařízení
        .route("/api/devices", get(api::list_devices))
        .route("/api/devices/:udid", axum::routing::delete(api::delete_device))
        // pairing upload (z CLI)
        .route("/api/pair", post(api::upload_pairing))
        // IPA katalog
        .route("/api/ipa", get(api::list_ipa).post(api::upload_ipa))
        .route("/api/ipa/:id", axum::routing::delete(api::delete_ipa))
        // instalace
        .route("/api/install", post(api::install))
        .route("/api/installations", get(api::list_installations))
        // úlohy
        .route("/api/jobs", get(api::list_jobs))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024 * 1024)) // 2 GB IPA
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
