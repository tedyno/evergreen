//! Apple ID account and Developer Services.
//!
//! Auth (GrandSlam/GSA + anisette) is done via `icloud_auth`. No Rust library provides
//! the Developer Portal API (device registration, cert, App ID, profile) — it lives in
//! `devportal` and is the main piece of M2, only testable with a real account.

pub mod devportal;

use tokio::sync::Mutex;

use icloud_auth::{AppleAccount, LoginState};
use omnisette::AnisetteConfiguration;

/// Decrypts an Apple GrandSlam GCM blob of the format "XYZ" + IV(16) + ciphertext + tag(16)
/// using the session key (AES-256-GCM, 16-byte nonce).
fn decrypt_gcm_xyz(sk: &[u8], et: &[u8]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::aes::Aes256;
    use aes_gcm::AesGcm;
    use aes_gcm::aead::consts::U16;

    if et.len() < 3 + 16 + 16 || &et[..3] != b"XYZ" {
        return Err(format!("neočekávaný formát et ({}B)", et.len()));
    }
    if sk.len() != 32 {
        return Err(format!("sk není 32B ({}B)", sk.len()));
    }
    use aes_gcm::aead::Payload;
    type Aes256Gcm16 = AesGcm<Aes256, U16>;
    let cipher = Aes256Gcm16::new(sk.into());
    let iv = &et[3..19];
    let ct_and_tag = &et[19..];
    // AAD = the 3-byte "XYZ" header.
    cipher
        .decrypt(iv.into(), Payload { msg: ct_and_tag, aad: &et[..3] })
        .map_err(|e| format!("{e}"))
}

/// Auth material for Developer Services (developerservices2.apple.com).
pub struct XcodeAuth {
    pub dsid: String,
    pub token: String,
    pub anisette: std::collections::HashMap<String, String>,
}

impl XcodeAuth {
    /// X-Apple-GS-Token = base64("dsid:token").
    pub fn gs_token_b64(&self) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine};
        STANDARD.encode(format!("{}:{}", self.dsid, self.token))
    }
}

/// Marker error: Apple rejected an app-token request with -22411 because the stored
/// session is no longer valid. Used so callers can trigger a re-login and retry.
#[derive(Debug)]
struct StaleSession;
impl std::fmt::Display for StaleSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Apple session vypršela (GSA -22411)")
    }
}
impl std::error::Error for StaleSession {}

/// Result of a login attempt.
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
    /// Cache of the Xcode app token (valid ~a year) — so we don't hit GSA on every request
    /// (otherwise Apple throttles with error -22411).
    xcode_token: Option<(String, i64)>, // (token, expiry_ms)
}

pub struct AppleClient {
    anisette_url: String,
    inner: Mutex<AuthInner>,
    /// Where to persist the Xcode token (survives restart → we don't touch GSA → no throttle).
    token_path: std::path::PathBuf,
}

impl AppleClient {
    pub fn new(anisette_url: String, data_dir: &std::path::Path) -> Self {
        let client = Self {
            anisette_url,
            inner: Mutex::new(AuthInner::default()),
            token_path: data_dir.join("xcode_token"),
        };
        client.load_persisted_token();
        client
    }

    /// Loads the persisted Xcode token from disk into the cache (format "expiry\ntoken").
    fn load_persisted_token(&self) {
        if let Ok(s) = std::fs::read_to_string(&self.token_path) {
            if let Some((exp, tok)) = s.split_once('\n') {
                if let Ok(exp) = exp.trim().parse::<i64>() {
                    // inner is not yet locked by anyone else (constructor).
                    if let Ok(mut inner) = self.inner.try_lock() {
                        inner.xcode_token = Some((tok.to_string(), exp));
                    }
                }
            }
        }
    }

    fn persist_token(&self, token: &str, expiry: i64) {
        let _ = std::fs::write(&self.token_path, format!("{expiry}\n{token}"));
    }

    fn anisette_config(&self) -> AnisetteConfiguration {
        AnisetteConfiguration::new().set_anisette_url(self.anisette_url.clone())
    }

    /// Step 1: log in with username+password. If the account requires 2FA, it sends the
    /// code to the trusted devices and returns `Needs2FA`.
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
                // Request the code on the trusted devices.
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

    /// Step 2: verify the 2FA code and complete the login.
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

        // After verification the login needs to be repeated to complete it.
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

    /// Serializes the logged-in session (spd: adsid, GsIdmsToken, …) as a plist XML,
    /// so it survives a server restart. None if we are not logged in.
    pub async fn export_session(&self) -> Option<String> {
        let inner = self.inner.lock().await;
        let spd = inner.account.as_ref()?.spd.as_ref()?;
        let mut buf: Vec<u8> = Vec::new();
        plist::to_writer_xml(&mut buf, spd).ok()?;
        String::from_utf8(buf).ok()
    }

    /// Restores the login from a stored session (after a server restart), without password/2FA.
    pub async fn restore_session(
        &self,
        apple_id: String,
        password: String,
        session_xml: &str,
    ) -> anyhow::Result<()> {
        let spd: plist::Dictionary = plist::from_bytes(session_xml.as_bytes())
            .map_err(|e| anyhow::anyhow!("parse session: {e}"))?;
        let mut account = AppleAccount::new(self.anisette_config())
            .await
            .map_err(|e| anyhow::anyhow!("anisette init: {e:?}"))?;
        account.spd = Some(spd);

        let mut inner = self.inner.lock().await;
        inner.apple_id = Some(apple_id);
        // Keep the password so a stale session can be refreshed automatically (-22411).
        inner.password = Some(password);
        inner.account = Some(account);
        inner.logged_in = true;
        Ok(())
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

    /// Auth material for Developer Services: DSID + Xcode token + anisette headers.
    /// The token is cached (valid ~a year) so Apple doesn't throttle repeated GSA requests.
    /// Gets Developer Services auth material, transparently re-logging in once if the
    /// stored session has gone stale (Apple answers app-token requests with -22411).
    pub async fn xcode_auth(&self) -> anyhow::Result<XcodeAuth> {
        match self.xcode_auth_once().await {
            Err(e) if e.downcast_ref::<StaleSession>().is_some() => {
                tracing::warn!("GSA session vypršela (-22411) — zkouším automatický re-login");
                self.reauth().await?;
                self.xcode_auth_once().await
            }
            other => other,
        }
    }

    /// Re-logs in with the stored credentials to refresh a stale session. Fails if the
    /// password isn't in memory or Apple demands a fresh 2FA (then the user must log in).
    async fn reauth(&self) -> anyhow::Result<()> {
        let (apple_id, password) = {
            let inner = self.inner.lock().await;
            match (inner.apple_id.clone(), inner.password.clone()) {
                (Some(a), Some(p)) => (a, p),
                _ => anyhow::bail!("automatický re-login nelze — přihlas se prosím znovu ručně"),
            }
        };
        match self.login(&apple_id, &password).await? {
            LoginOutcome::LoggedIn { .. } => Ok(()),
            LoginOutcome::Needs2FA => {
                anyhow::bail!("re-login vyžaduje 2FA — potvrď kód v Účtu a přihlas se znovu")
            }
        }
    }

    /// One attempt to obtain the Xcode auth token (wrapped by `xcode_auth` for reauth).
    async fn xcode_auth_once(&self) -> anyhow::Result<XcodeAuth> {
        let mut inner = self.inner.lock().await;
        let account = inner
            .account
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("nepřihlášeno"))?;

        let dsid = account
            .spd
            .as_ref()
            .and_then(|s| s.get("adsid"))
            .and_then(|v| v.as_string())
            .ok_or_else(|| anyhow::anyhow!("chybí adsid"))?
            .to_string();

        // Anisette headers are time-sensitive → generate fresh ones every time.
        let anisette = account.get_anisette().await.base_headers.clone();

        // Token cache (with a 1 h margin before expiration).
        let now_ms = chrono::Utc::now().timestamp_millis();
        if let Some((tok, exp)) = &inner.xcode_token {
            if *exp - 3_600_000 > now_ms {
                return Ok(XcodeAuth { dsid, token: tok.clone(), anisette });
            }
        }

        let account = inner.account.as_ref().unwrap();
        let token = match account.get_app_token("com.apple.gs.xcode.auth").await {
            Ok(t) => t,
            // -22411 = the restored session is no longer valid for app-token requests;
            // surface it as a typed marker so `xcode_auth` can re-login and retry.
            Err(icloud_auth::Error::AuthSrpWithMessage(-22411, _)) => {
                return Err(StaleSession.into())
            }
            Err(e) => return Err(anyhow::anyhow!("app token: {e:?}")),
        };
        let et = token.app_tokens.get("et").and_then(|v| v.as_data())
            .ok_or_else(|| anyhow::anyhow!("chybí et"))?;
        let sk = token.app_tokens.get("sk").and_then(|v| v.as_data())
            .ok_or_else(|| anyhow::anyhow!("chybí sk"))?;
        let dec = decrypt_gcm_xyz(sk, et).map_err(|e| anyhow::anyhow!("GCM: {e}"))?;
        let parsed: plist::Value = plist::from_bytes(&dec)?;
        let entry = parsed
            .as_dictionary()
            .and_then(|d| d.get("t"))
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("com.apple.gs.xcode.auth"))
            .and_then(|v| v.as_dictionary())
            .ok_or_else(|| anyhow::anyhow!("v tokenu chybí xcode.auth"))?;
        let xcode_token = entry
            .get("token")
            .and_then(|v| v.as_string())
            .ok_or_else(|| anyhow::anyhow!("chybí token"))?
            .to_string();
        let expiry = entry.get("expiry").and_then(|v| v.as_signed_integer())
            .unwrap_or(now_ms + 3_600_000);

        inner.xcode_token = Some((xcode_token.clone(), expiry));
        self.persist_token(&xcode_token, expiry);
        Ok(XcodeAuth { dsid, token: xcode_token, anisette })
    }

    /// Overview of the account's App IDs (teamId + account type + global list).
    pub async fn account_app_ids(
        &self,
    ) -> anyhow::Result<(String, bool, Vec<devportal::AppIdEntry>)> {
        let auth = self.xcode_auth().await?;
        devportal::account_app_ids(&auth).await
    }
}
