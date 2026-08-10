//! Developer Services API client (developerservices2.apple.com) — the plist/XML
//! protocol that Xcode uses. Auth via X-Apple-GS-Token (from `XcodeAuth`).

#![allow(dead_code)]

use std::collections::HashMap;

use anyhow::{anyhow, Result};

use super::XcodeAuth;

pub const BASE: &str = "https://developerservices2.apple.com/services/QH65B2";
pub const PROTOCOL_VERSION: &str = "QH65B2";
pub const CLIENT_ID: &str = "XABBG36SBA";

/// An App ID on the account (from listAppIds).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppIdEntry {
    pub app_id_id: String,
    pub identifier: String,   // bundle id
    pub name: String,
    pub expiration: Option<String>,
}

/// Sends a plist request to a Developer Services action and returns the response as a plist.
async fn request(
    auth: &XcodeAuth,
    action: &str,
    query: &[(&str, &str)],
    params: plist::Dictionary,
) -> Result<plist::Dictionary> {
    let mut body = params;
    body.insert("clientId".into(), CLIENT_ID.into());
    body.insert("protocolVersion".into(), PROTOCOL_VERSION.into());
    body.insert(
        "requestId".into(),
        uuid::Uuid::new_v4().to_string().to_uppercase().into(),
    );

    let mut xml: Vec<u8> = Vec::new();
    plist::to_writer_xml(&mut xml, &plist::Value::Dictionary(body))?;

    let url = format!("{BASE}/{action}");
    let client = reqwest::Client::new();
    let mut req = client
        .post(&url)
        .query(query)
        .header("Content-Type", "text/x-xml-plist")
        .header("Accept", "text/x-xml-plist")
        .header("User-Agent", "Xcode")
        .header("X-Apple-I-Identity-Id", &auth.dsid)
        .header("X-Apple-GS-Token", &auth.token)
        .header("X-Apple-App-Info", "com.apple.gs.xcode.auth")
        .header("X-Xcode-Version", "14.2 (14C18)");

    // Anisette headers from AOSKit.
    for (k, v) in &auth.anisette {
        req = req.header(k.as_str(), v.as_str());
    }
    // Additional headers that AOSKit doesn't provide.
    let now = chrono::Utc::now();
    req = req
        .header("X-Apple-I-Client-Time", now.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .header("X-Apple-I-TimeZone", "UTC")
        .header("X-Apple-Locale", "en_US");

    let resp = req.body(xml).send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    let dict: plist::Dictionary = plist::from_bytes(text.as_bytes()).map_err(|e| {
        let snippet: String = text.chars().take(400).collect();
        anyhow!("odpověď není plist (HTTP {status}): {e}; tělo: {snippet}")
    })?;

    // Developer Services error fields.
    if let Some(rc) = dict.get("resultCode").and_then(|v| v.as_signed_integer()) {
        if rc != 0 {
            let msg = dict
                .get("userString")
                .or_else(|| dict.get("resultString"))
                .and_then(|v| v.as_string())
                .unwrap_or("neznámá chyba");
            return Err(anyhow!("Developer Services chyba {rc}: {msg}"));
        }
    }
    Ok(dict)
}

/// Returns the teamId of the first team (a free account typically has one).
pub async fn first_team_id(auth: &XcodeAuth) -> Result<String> {
    let resp = request(auth, "listTeams.action", &[], plist::Dictionary::new()).await?;
    let teams = resp
        .get("teams")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("odpověď listTeams nemá teams"))?;
    let first = teams.first().and_then(|v| v.as_dictionary())
        .ok_or_else(|| anyhow!("žádný tým na účtu"))?;
    let team_id = first.get("teamId").and_then(|v| v.as_string())
        .ok_or_else(|| anyhow!("tým nemá teamId"))?;
    Ok(team_id.to_string())
}

/// List of App IDs on the account (global — including ones created by AltStore etc.).
pub async fn list_app_ids(auth: &XcodeAuth, team_id: &str) -> Result<Vec<AppIdEntry>> {
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    let resp = request(auth, "ios/listAppIds.action", &[("teamId", team_id)], params).await?;

    let app_ids = resp
        .get("appIds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("odpověď listAppIds nemá appIds"))?;

    let mut out = Vec::new();
    for a in app_ids {
        let Some(d) = a.as_dictionary() else { continue };
        let get = |k: &str| d.get(k).and_then(|v| v.as_string()).map(|s| s.to_string());
        out.push(AppIdEntry {
            app_id_id: get("appIdId").unwrap_or_default(),
            identifier: get("identifier").unwrap_or_default(),
            name: get("name").unwrap_or_default(),
            expiration: get("expirationDate"),
        });
    }
    Ok(out)
}

/// Complete overview of the account's App IDs (teamId + list) — for the app.
pub async fn account_app_ids(auth: &XcodeAuth) -> Result<(String, Vec<AppIdEntry>)> {
    let team = first_team_id(auth).await?;
    let ids = list_app_ids(auth, &team).await?;
    Ok((team, ids))
}

// ------------------------------------------------------------- devices

/// Registers a device UDID (idempotently — Apple returns the existing one).
pub async fn add_device(auth: &XcodeAuth, team_id: &str, udid: &str, name: &str) -> Result<String> {
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    params.insert("deviceNumber".into(), udid.into());
    params.insert("name".into(), name.into());
    let resp = request(auth, "ios/addDevice.action", &[("teamId", team_id)], params).await;
    // If the device already exists, Apple returns an error — we take it from listDevices.
    match resp {
        Ok(d) => Ok(d
            .get("device")
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("deviceId"))
            .and_then(|v| v.as_string())
            .unwrap_or_default()
            .to_string()),
        Err(add_err) => {
            let devs = list_devices(auth, team_id)
                .await
                .map_err(|list_err| anyhow!("addDevice: {add_err:#}; listDevices: {list_err:#}"))?;
            devs.into_iter()
                .find(|(u, _)| u.eq_ignore_ascii_case(udid))
                .map(|(_, id)| id)
                .ok_or_else(|| anyhow!("zařízení nezaregistrováno; addDevice chyba: {add_err:#}"))
        }
    }
}

/// (UDID, deviceId) of registered devices.
pub async fn list_devices(auth: &XcodeAuth, team_id: &str) -> Result<Vec<(String, String)>> {
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    let resp = request(auth, "ios/listDevices.action", &[("teamId", team_id)], params).await?;
    let arr = resp.get("devices").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|v| {
            let d = v.as_dictionary()?;
            Some((
                d.get("deviceNumber")?.as_string()?.to_string(),
                d.get("deviceId")?.as_string()?.to_string(),
            ))
        })
        .collect())
}

// ------------------------------------------------------------- certificate

/// Development certificate (DER + serial) from the account.
#[derive(Debug, Clone)]
pub struct DevCert {
    pub cert_der: Vec<u8>,
    pub serial_number: String,
}

/// Submits a CSR (PEM) and returns the development certificate.
pub async fn submit_csr(
    auth: &XcodeAuth,
    team_id: &str,
    csr_pem: &str,
    machine_id: &str,
    machine_name: &str,
) -> Result<DevCert> {
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    params.insert("csrContent".into(), csr_pem.into());
    params.insert("machineId".into(), machine_id.into());
    params.insert("machineName".into(), machine_name.into());
    let resp = request(auth, "ios/submitDevelopmentCSR.action", &[("teamId", team_id)], params).await?;

    let cr = resp
        .get("certRequest")
        .and_then(|v| v.as_dictionary())
        .ok_or_else(|| anyhow!("odpověď submitCSR nemá certRequest"))?;
    let serial = cr.get("serialNumber").and_then(|v| v.as_string()).unwrap_or_default().to_string();
    if let Some(der) = cr.get("certContent").and_then(|v| v.as_data()) {
        return Ok(DevCert { cert_der: der.to_vec(), serial_number: serial });
    }
    // Sometimes the cert is only in listAllDevelopmentCerts, keyed by serial.
    let certs = list_dev_certs(auth, team_id).await?;
    certs
        .into_iter()
        .find(|c| c.serial_number == serial || serial.is_empty())
        .ok_or_else(|| anyhow!("cert po CSR nenalezen"))
}

/// List of development certificates (DER + serial).
pub async fn list_dev_certs(auth: &XcodeAuth, team_id: &str) -> Result<Vec<DevCert>> {
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    let resp = request(auth, "ios/listAllDevelopmentCerts.action", &[("teamId", team_id)], params).await?;
    let arr = resp.get("certificates").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|v| {
            let d = v.as_dictionary()?;
            let der = d.get("certContent")?.as_data()?.to_vec();
            let serial = d.get("serialNumber").and_then(|x| x.as_string()).unwrap_or("").to_string();
            Some(DevCert { cert_der: der, serial_number: serial })
        })
        .collect())
}

// ------------------------------------------------------------- App ID + profile

/// Creates an App ID for the bundle id (idempotently — returns the existing appIdId).
pub async fn ensure_app_id(auth: &XcodeAuth, team_id: &str, bundle_id: &str, name: &str) -> Result<String> {
    // First try to find an existing one.
    let existing = list_app_ids(auth, team_id).await?;
    if let Some(a) = existing.iter().find(|a| a.identifier == bundle_id) {
        return Ok(a.app_id_id.clone());
    }
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    params.insert("identifier".into(), bundle_id.into());
    params.insert("name".into(), sanitize_app_name(name).into());
    let resp = request(auth, "ios/addAppId.action", &[("teamId", team_id)], params).await?;
    resp.get("appId")
        .and_then(|v| v.as_dictionary())
        .and_then(|d| d.get("appIdId"))
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("odpověď addAppId nemá appIdId"))
}

/// Downloads the team provisioning profile for the App ID (contains all the team's devices).
pub async fn download_profile(auth: &XcodeAuth, team_id: &str, app_id_id: &str) -> Result<Vec<u8>> {
    let mut params = plist::Dictionary::new();
    params.insert("teamId".into(), team_id.into());
    params.insert("appIdId".into(), app_id_id.into());
    let resp = request(
        auth,
        "ios/downloadTeamProvisioningProfile.action",
        &[("teamId", team_id)],
        params,
    )
    .await?;
    resp.get("provisioningProfile")
        .and_then(|v| v.as_dictionary())
        .and_then(|d| d.get("encodedProfile"))
        .and_then(|v| v.as_data())
        .map(|d| d.to_vec())
        .ok_or_else(|| anyhow!("odpověď downloadProfile nemá encodedProfile"))
}

/// The App ID name may only be alphanumeric + spaces.
fn sanitize_app_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect();
    let s = s.trim();
    if s.is_empty() { "Homesign App".to_string() } else { s.to_string() }
}

/// Left as M2 (needs further tuning against the account and device).
pub struct SigningIdentity {
    pub certificate_der: Vec<u8>,
    pub private_key_pem: String,
    pub serial_number: String,
}

pub struct Profile {
    pub data: Vec<u8>,
    pub app_id: String,
    pub expiration: chrono::DateTime<chrono::Utc>,
}

pub struct DevPortalClient {
    team_id: String,
}

impl DevPortalClient {
    pub fn new(team_id: String) -> Self {
        Self { team_id }
    }

    pub async fn register_device(&self, _udid: &str, _name: &str) -> Result<()> {
        anyhow::bail!("devportal::register_device: M2")
    }
    pub async fn ensure_certificate(&self) -> Result<SigningIdentity> {
        anyhow::bail!("devportal::ensure_certificate: M2")
    }
    pub async fn ensure_app_id(&self, _bundle_id: &str) -> Result<String> {
        anyhow::bail!("devportal::ensure_app_id: M2")
    }
    pub async fn download_profile(&self, _app_id: &str, _udid: &str) -> Result<Profile> {
        anyhow::bail!("devportal::download_profile: M2")
    }
}

/// Unused, but keeps the HashMap import readable for future extension.
pub type Headers = HashMap<String, String>;
