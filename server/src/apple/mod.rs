//! Apple ID účet a Developer Services.
//!
//! Auth (GrandSlam/GSA + anisette) je hotové přes `icloud_auth`. Developer Portal
//! API (registrace zařízení, cert, App ID, profil) žádná Rust knihovna neposkytuje
//! — je v `devportal` a je to hlavní kus M2, testovatelný až s reálným účtem.

pub mod devportal;

use tokio::sync::Mutex;

use icloud_auth::{AppleAccount, LoginState};
use omnisette::AnisetteConfiguration;

/// Dešifruje Apple GrandSlam GCM blob formátu "XYZ" + IV(16) + ciphertext + tag(16)
/// pomocí session klíče (AES-256-GCM, 16bajtový nonce).
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
    // AAD = 3bajtová hlavička "XYZ".
    cipher
        .decrypt(iv.into(), Payload { msg: ct_and_tag, aad: &et[..3] })
        .map_err(|e| format!("{e}"))
}

/// Auth materiál pro Developer Services (developerservices2.apple.com).
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
    /// Cache Xcode app tokenu (platí ~rok) — ať netlučeme GSA při každém requestu
    /// (Apple jinak throttluje chybou -22411).
    xcode_token: Option<(String, i64)>, // (token, expiry_ms)
}

pub struct AppleClient {
    anisette_url: String,
    inner: Mutex<AuthInner>,
    /// Kam persistovat Xcode token (přežije restart → neťukáme GSA → žádný throttle).
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

    /// Načte perzistovaný Xcode token z disku do cache (formát "expiry\ntoken").
    fn load_persisted_token(&self) {
        if let Ok(s) = std::fs::read_to_string(&self.token_path) {
            if let Some((exp, tok)) = s.split_once('\n') {
                if let Ok(exp) = exp.trim().parse::<i64>() {
                    // inner ještě není zamčené nikým jiným (konstruktor).
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

    /// Serializuje přihlášenou session (spd: adsid, GsIdmsToken, …) jako plist XML,
    /// ať přežije restart serveru. None, když nejsme přihlášení.
    pub async fn export_session(&self) -> Option<String> {
        let inner = self.inner.lock().await;
        let spd = inner.account.as_ref()?.spd.as_ref()?;
        let mut buf: Vec<u8> = Vec::new();
        plist::to_writer_xml(&mut buf, spd).ok()?;
        String::from_utf8(buf).ok()
    }

    /// Obnoví přihlášení z uložené session (po restartu serveru), bez hesla/2FA.
    pub async fn restore_session(&self, apple_id: String, session_xml: &str) -> anyhow::Result<()> {
        let spd: plist::Dictionary = plist::from_bytes(session_xml.as_bytes())
            .map_err(|e| anyhow::anyhow!("parse session: {e}"))?;
        let mut account = AppleAccount::new(self.anisette_config())
            .await
            .map_err(|e| anyhow::anyhow!("anisette init: {e:?}"))?;
        account.spd = Some(spd);

        let mut inner = self.inner.lock().await;
        inner.apple_id = Some(apple_id);
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

    /// Auth materiál pro Developer Services: DSID + Xcode token + anisette hlavičky.
    /// Token se cachuje (platí ~rok), ať Apple nethrottluje opakované GSA requesty.
    pub async fn xcode_auth(&self) -> anyhow::Result<XcodeAuth> {
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

        // Anisette hlavičky jsou časově citlivé → generuj čerstvé pokaždé.
        let anisette = account.get_anisette().await.base_headers.clone();

        // Cache tokenu (s rezervou 1 h před expirací).
        let now_ms = chrono::Utc::now().timestamp_millis();
        if let Some((tok, exp)) = &inner.xcode_token {
            if *exp - 3_600_000 > now_ms {
                return Ok(XcodeAuth { dsid, token: tok.clone(), anisette });
            }
        }

        let account = inner.account.as_ref().unwrap();
        let token = account
            .get_app_token("com.apple.gs.xcode.auth")
            .await
            .map_err(|e| anyhow::anyhow!("app token: {e:?}"))?;
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

    /// Přehled App ID účtu (teamId + globální seznam) přes Developer Services.
    pub async fn account_app_ids(
        &self,
    ) -> anyhow::Result<(String, Vec<devportal::AppIdEntry>)> {
        let auth = self.xcode_auth().await?;
        devportal::account_app_ids(&auth).await
    }
}
