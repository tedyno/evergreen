//! Sdílený stav aplikace předávaný do handlerů.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

use crate::apple::AppleClient;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: SqlitePool,
    /// Apple ID klient — drží přihlašovací stav mezi requesty (2FA flow).
    pub apple: Arc<AppleClient>,
    /// Abort handles běžících úloh — pro zrušení z UI.
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
