//! Sdílený stav aplikace předávaný do handlerů.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::apple::AppleClient;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: SqlitePool,
    /// Apple ID klient — drží přihlašovací stav mezi requesty (2FA flow).
    pub apple: Arc<AppleClient>,
}

impl AppState {
    pub fn new(cfg: Config, db: SqlitePool) -> Self {
        let anisette_url = cfg.anisette_url.clone();
        Self {
            cfg: Arc::new(cfg),
            db,
            apple: Arc::new(AppleClient::new(anisette_url)),
        }
    }
}
