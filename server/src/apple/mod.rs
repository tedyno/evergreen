//! Apple ID účet a Developer Services.
//!
//! Auth (GrandSlam/GSA + anisette) je hotové přes `icloud_auth`. Developer Portal
//! API (registrace zařízení, cert, App ID, profil) žádná Rust knihovna neposkytuje
//! — je v `devportal` a je to hlavní kus M2, testovatelný až s reálným účtem.

pub mod devportal;

use tokio::sync::Mutex;

use icloud_auth::{AppleAccount, LoginState};
use omnisette::AnisetteConfiguration;

/// Výsledek pokusu o přihlášení.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state")]
pub enum LoginOutcome {
    #[serde(rename = "logged_in")]
    LoggedIn { team_id: Option<String> },
    #[serde(rename = "needs_2fa")]
    Needs2FA,
}

#[derive(Default)]
struct AuthInner {
    account: Option<AppleAccount>,
    apple_id: Option<String>,
    password: Option<String>,
    logged_in: bool,
}

pub struct AppleClient {
    anisette_url: String,
    inner: Mutex<AuthInner>,
}

impl AppleClient {
    pub fn new(anisette_url: String) -> Self {
        Self { anisette_url, inner: Mutex::new(AuthInner::default()) }
    }

    fn anisette_config(&self) -> AnisetteConfiguration {
        AnisetteConfiguration::new().set_anisette_url(self.anisette_url.clone())
    }

    /// Krok 1: přihlášení jménem+heslem. Když účet vyžaduje 2FA, pošle kód na
    /// důvěryhodná zařízení a vrátí `Needs2FA`.
    pub async fn login(&self, apple_id: &str, password: &str) -> anyhow::Result<LoginOutcome> {
        let mut account = AppleAccount::new(self.anisette_config())
            .await
            .map_err(|e| anyhow::anyhow!("anisette/auth init: {e:?}"))?;

        let state = account
            .login_email_pass(apple_id, password)
            .await
            .map_err(|e| anyhow::anyhow!("login: {e:?}"))?;

        let mut inner = self.inner.lock().await;
        inner.apple_id = Some(apple_id.to_string());
        inner.password = Some(password.to_string());

        match state {
            LoginState::LoggedIn => {
                inner.logged_in = true;
                inner.account = Some(account);
                Ok(LoginOutcome::LoggedIn { team_id: None })
            }
            LoginState::NeedsDevice2FA | LoginState::Needs2FAVerification => {
                // Vyžádej kód na důvěryhodná zařízení.
                account
                    .send_2fa_to_devices()
                    .await
                    .map_err(|e| anyhow::anyhow!("2fa request: {e:?}"))?;
                inner.account = Some(account);
                Ok(LoginOutcome::Needs2FA)
            }
            other => {
                inner.account = Some(account);
                Err(anyhow::anyhow!("nepodporovaný stav přihlášení: {other:?}"))
            }
        }
    }

    /// Krok 2: ověření 2FA kódu a dokončení přihlášení.
    pub async fn submit_2fa(&self, code: &str) -> anyhow::Result<LoginOutcome> {
        let mut inner = self.inner.lock().await;
        let (apple_id, password) = match (inner.apple_id.clone(), inner.password.clone()) {
            (Some(a), Some(p)) => (a, p),
            _ => anyhow::bail!("nejdřív zavolej login"),
        };
        let mut account = inner
            .account
            .take()
            .ok_or_else(|| anyhow::anyhow!("žádná probíhající session"))?;

        account
            .verify_2fa(code.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("ověření 2fa: {e:?}"))?;

        // Po ověření je potřeba zopakovat login, aby se dokončil.
        let state = account
            .login_email_pass(&apple_id, &password)
            .await
            .map_err(|e| anyhow::anyhow!("dokončení login: {e:?}"))?;

        match state {
            LoginState::LoggedIn => {
                inner.logged_in = true;
                inner.account = Some(account);
                Ok(LoginOutcome::LoggedIn { team_id: None })
            }
            other => {
                inner.account = Some(account);
                Err(anyhow::anyhow!("po 2fa nečekaný stav: {other:?}"))
            }
        }
    }

    pub async fn logout(&self) {
        let mut inner = self.inner.lock().await;
        *inner = AuthInner::default();
    }

    pub async fn status(&self) -> &'static str {
        let inner = self.inner.lock().await;
        if inner.logged_in {
            "logged_in"
        } else if inner.account.is_some() {
            "needs_2fa"
        } else {
            "logged_out"
        }
    }

    pub async fn is_logged_in(&self) -> bool {
        self.inner.lock().await.logged_in
    }
}
