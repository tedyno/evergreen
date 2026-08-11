//! HTTP router: web UI + REST API + static files (IPA icons).

mod api;
mod ui;

use axum::extract::DefaultBodyLimit;
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
        .route("/api/appids", get(api::account_appids))
        .route("/api/debug/install-direct", post(api::debug_install_direct))
        // account
        .route("/api/account", get(api::account_status))
        .route("/api/account/login", post(api::login))
        .route("/api/account/2fa", post(api::submit_2fa))
        .route("/api/account/logout", post(api::logout))
        // devices
        .route("/api/devices", get(api::list_devices))
        .route("/api/devices/:udid", axum::routing::delete(api::delete_device))
        .route("/api/devices/:udid/address", post(api::set_device_address))
        .route("/api/devices/:udid/detect-ip", post(api::detect_device_ip))
        // pairing the iPad over USB directly in the server (no CLI)
        .route("/api/pair/usb", get(api::pair_usb_list).post(api::pair_usb))
        // pairing upload (backward compatibility)
        .route("/api/pair", post(api::upload_pairing))
        // IPA catalog
        .route("/api/ipa", get(api::list_ipa).post(api::upload_ipa))
        .route("/api/ipa/:id", axum::routing::delete(api::delete_ipa))
        // installation
        .route("/api/install", post(api::install))
        .route("/api/installations", get(api::list_installations))
        // jobs
        .route("/api/jobs", get(api::list_jobs))
        .route("/api/jobs/:id/cancel", post(api::cancel_job))
        .route("/api/refresh/run", post(api::refresh_now))
        // axum has its own DefaultBodyLimit (2 MB) on extractors — without this,
        // the multipart IPA upload fails with a reset/400. The hard cap is held by the layer below.
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024 * 1024)) // 2 GB IPA
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
