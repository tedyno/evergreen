//! Konfigurace serveru z prostředí (12-factor). Vše má rozumný default pro
//! běh v domácí síti / Dockeru.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    /// Kam bindovat HTTP server (web UI + API).
    pub bind: SocketAddr,
    /// Datový adresář — SQLite DB, nahrané IPA, pairing files.
    pub data_dir: PathBuf,
    /// URL anisette (omnisette) sidecaru pro Apple ID auth.
    pub anisette_url: String,
    /// Klíč pro šifrování citlivých polí v DB (Apple ID heslo, session tokeny).
    /// Base64, 32 bajtů. Když chybí, vygeneruje se a uloží do data_dir/master.key.
    pub master_key: [u8; 32],
    /// Kolik dní před expirací profilu spustit refresh.
    pub refresh_before_days: i64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        // Loopback default — server nemá auth vrstvu, nesmí viset na celé síti.
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
    // Klíč dešifruje Apple ID heslo + session — jen pro vlastníka (0600).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::info!("vygenerován nový master.key v {}", path.display());
    Ok(key)
}
