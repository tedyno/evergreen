//! Domain types mirroring the tables in migrations/0001_init.sql.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub address: Option<String>,
    #[serde(skip)]
    pub pairing_path: String,
    pub model: Option<String>,
    pub ios_version: Option<String>,
    pub created_at: String,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Ipa {
    pub id: String,
    pub filename: String,
    pub bundle_id: String,
    pub name: String,
    pub version: Option<String>,
    pub size_bytes: i64,
    #[serde(skip)]
    pub path: String,
    pub icon_path: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Installation {
    pub id: i64,
    pub device_udid: String,
    pub ipa_id: String,
    pub signed_bundle_id: String,
    pub app_id_ext: Option<String>,
    pub profile_expires: Option<String>,
    pub last_installed: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

impl Installation {
    /// When the profile expires (None = unknown / no expiration).
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.profile_expires
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub device_udid: Option<String>,
    pub ipa_id: Option<String>,
    pub status: String,
    pub progress: i64,
    pub message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallRequest {
    pub device_udid: String,
    pub ipa_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub apple_id: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TwoFactorRequest {
    pub code: String,
}
