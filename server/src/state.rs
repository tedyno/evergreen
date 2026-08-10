//! Shared application state passed into the handlers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::SqlitePool;
use tokio::task::AbortHandle;

use crate::apple::AppleClient;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: SqlitePool,
    /// Apple ID client — holds the login state across requests (2FA flow).
    pub apple: Arc<AppleClient>,
    /// Abort handles of running jobs — for cancellation from the UI.
    pub running_jobs: Arc<Mutex<HashMap<i64, AbortHandle>>>,
}

impl AppState {
    pub fn new(cfg: Config, db: SqlitePool) -> Self {
        let apple = AppleClient::new(cfg.anisette_url.clone(), &cfg.data_dir);
        Self {
            cfg: Arc::new(cfg),
            db,
            apple: Arc::new(apple),
            running_jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
