//! Developer Services API klient (developerservices2.apple.com) — plist/XML
//! protokol, který používá Xcode. Registrace zařízení, CSR→certifikát, App ID,
//! provisioning profil. Žádná Rust knihovna to nedělá; endpointy jsou známé
//! z AltSign.
//!
//! STAV: M2. Struktura a endpointy jsou připravené, ale celý tok se dá ověřit
//! jen s reálným (přihlášeným) Apple ID a zařízením. Metody proto zatím vrací
//! `not_implemented`, dokud je ráno nedoladíme proti živému účtu — nechci
//! předstírat funkčnost, kterou nejde otestovat.

#![allow(dead_code)]

pub const BASE: &str = "https://developerservices2.apple.com/services/QH65B2";
pub const PROTOCOL_VERSION: &str = "QH65B2";

/// Certifikát + privátní klíč pro podepisování.
pub struct SigningIdentity {
    pub certificate_der: Vec<u8>,
    pub private_key_pem: String,
    pub serial_number: String,
}

/// Provisioning profil stažený z portálu.
pub struct Profile {
    pub data: Vec<u8>,          // embedded.mobileprovision
    pub app_id: String,         // identifier na portálu
    pub expiration: chrono::DateTime<chrono::Utc>,
}

pub struct DevPortalClient {
    http: reqwest::Client,
    team_id: String,
}

impl DevPortalClient {
    pub fn new(team_id: String) -> Self {
        Self { http: reqwest::Client::new(), team_id }
    }

    /// GET /listTeams — vybere první tým (free účet má typicky jeden).
    pub async fn fetch_first_team(_auth_token: &str) -> anyhow::Result<String> {
        anyhow::bail!("devportal::fetch_first_team: M2, čeká na test s živým účtem")
    }

    /// Zaregistruje UDID zařízení (idempotentně — vrací existující, když už je).
    pub async fn register_device(&self, _udid: &str, _name: &str) -> anyhow::Result<()> {
        anyhow::bail!("devportal::register_device: M2")
    }

    /// Vytvoří (nebo znovupoužije) development certifikát z CSR.
    pub async fn ensure_certificate(&self) -> anyhow::Result<SigningIdentity> {
        anyhow::bail!("devportal::ensure_certificate: M2")
    }

    /// Zajistí App ID pro daný bundle id (recykluje kvůli limitu 10/týden).
    pub async fn ensure_app_id(&self, _bundle_id: &str) -> anyhow::Result<String> {
        anyhow::bail!("devportal::ensure_app_id: M2")
    }

    /// Stáhne provisioning profil pro App ID + zařízení.
    pub async fn download_profile(
        &self,
        _app_id: &str,
        _udid: &str,
    ) -> anyhow::Result<Profile> {
        anyhow::bail!("devportal::download_profile: M2")
    }
}
