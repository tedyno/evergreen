//! Server configuration from the environment (12-factor). Everything has a sane
//! default for running on a home network / in Docker.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    /// Where to bind the HTTP server (web UI + API).
    pub bind: SocketAddr,
    /// Data directory — SQLite DB, uploaded IPAs, pairing files.
    pub data_dir: PathBuf,
    /// URL of the anisette (omnisette) sidecar for Apple ID auth.
    pub anisette_url: String,
    /// Key for encrypting sensitive fields in the DB (Apple ID password, session tokens).
    /// Base64, 32 bytes. If missing, it is generated and stored in data_dir/master.key.
    pub master_key: [u8; 32],
    /// How many days before profile expiration to trigger a refresh.
    pub refresh_before_days: i64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        // Loopback default — the server has no auth layer, so it must not listen on the whole network.
        let bind = std::env::var("HOMESIGN_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()?;
        let data_dir = PathBuf::from(
            std::env::var("HOMESIGN_DATA").unwrap_or_else(|_| "./data".to_string()),
        );
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(data_dir.join("ipa"))?;
        std::fs::create_dir_all(data_dir.join("pairing"))?;

        let anisette_url = std::env::var("ANISETTE_URL")
            .unwrap_or_else(|_| "http://anisette:6969".to_string());

        let master_key = load_or_create_master_key(&data_dir)?;

        let refresh_before_days = std::env::var("HOMESIGN_REFRESH_BEFORE_DAYS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        Ok(Self { bind, data_dir, anisette_url, master_key, refresh_before_days })
    }

    pub fn db_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.data_dir.join("homesign.db").display())
    }

    pub fn ipa_dir(&self) -> PathBuf {
        self.data_dir.join("ipa")
    }

    pub fn pairing_dir(&self) -> PathBuf {
        self.data_dir.join("pairing")
    }
}

fn load_or_create_master_key(data_dir: &std::path::Path) -> anyhow::Result<[u8; 32]> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let path = data_dir.join("master.key");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let raw = STANDARD.decode(existing.trim())?;
        if raw.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&raw);
            return Ok(key);
        }
    }
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    std::fs::write(&path, STANDARD.encode(key))?;
    // The key decrypts the Apple ID password + session — owner-only (0600).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::info!("vygenerován nový master.key v {}", path.display());
    Ok(key)
}
