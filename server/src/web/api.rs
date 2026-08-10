//! REST API handlery.

use axum::extract::{Multipart, Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::apple::LoginOutcome;
use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::state::AppState;
use crate::{ipa as ipautil, jobs};

pub async fn status() -> Json<Value> {
    Json(json!({
        "name": "homesign",
        "version": env!("CARGO_PKG_VERSION"),
        "ok": true,
    }))
}

// ---------------------------------------------------------------- účet

pub async fn account_status(State(st): State<AppState>) -> AppResult<Json<Value>> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT apple_id, team_id FROM account WHERE id = 1")
            .fetch_optional(&st.db)
            .await?;
    let auth = st.apple.status().await;
    Ok(Json(json!({
        "linked": row.is_some(),
        "apple_id": row.as_ref().map(|r| r.0.clone()),
        "team_id": row.and_then(|r| r.1),
        "auth_state": auth,
    })))
}

pub async fn login(
    State(st): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<Value>> {
    let outcome = st
        .apple
        .login(&req.apple_id, &req.password)
        .await
        .map_err(AppError::Other)?;

    // Ulož účet (heslo šifrovaně) hned po prvním pokusu.
    let pw_enc = crate::crypto::encrypt(&st.cfg.master_key, &req.password)
        .map_err(AppError::Other)?;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO account (id, apple_id, password_enc, updated_at) VALUES (1, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET apple_id = excluded.apple_id,
             password_enc = excluded.password_enc, updated_at = excluded.updated_at",
    )
    .bind(&req.apple_id)
    .bind(&pw_enc)
    .bind(&now)
    .execute(&st.db)
    .await?;

    persist_session(&st).await;
    Ok(Json(outcome_json(&outcome)))
}

/// Uloží přihlášenou session šifrovaně do DB, ať přežije restart serveru.
async fn persist_session(st: &AppState) {
    if let Some(sess) = st.apple.export_session().await {
        if let Ok(enc) = crate::crypto::encrypt(&st.cfg.master_key, &sess) {
            let now = chrono::Utc::now().to_rfc3339();
            let _ = sqlx::query("UPDATE account SET session_enc = ?, updated_at = ? WHERE id = 1")
                .bind(&enc)
                .bind(&now)
                .execute(&st.db)
                .await;
        }
    }
}

pub async fn submit_2fa(
    State(st): State<AppState>,
    Json(req): Json<TwoFactorRequest>,
) -> AppResult<Json<Value>> {
    let outcome = st.apple.submit_2fa(&req.code).await.map_err(AppError::Other)?;
    if let LoginOutcome::LoggedIn { ref team_id } = outcome {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("UPDATE account SET team_id = ?, updated_at = ? WHERE id = 1")
            .bind(team_id)
            .bind(&now)
            .execute(&st.db)
            .await?;
        persist_session(&st).await;
    }
    Ok(Json(outcome_json(&outcome)))
}

pub async fn logout(State(st): State<AppState>) -> AppResult<Json<Value>> {
    st.apple.logout().await;
    Ok(Json(json!({ "ok": true })))
}

fn outcome_json(o: &LoginOutcome) -> Value {
    match o {
        LoginOutcome::LoggedIn { team_id } => json!({ "state": "logged_in", "team_id": team_id }),
        LoginOutcome::Needs2FA => json!({ "state": "needs_2fa" }),
    }
}

// ------------------------------------------------------------- zařízení

pub async fn list_devices(State(st): State<AppState>) -> AppResult<Json<Vec<Device>>> {
    let devices = sqlx::query_as::<_, Device>("SELECT * FROM device ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    Ok(Json(devices))
}

pub async fn delete_device(
    State(st): State<AppState>,
    Path(udid): Path<String>,
) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM device WHERE udid = ?")
        .bind(&udid)
        .execute(&st.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// Upload pairing file z CLI. Tělo = plist pairing file, hlavičky nesou metadata.
pub async fn upload_pairing(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> AppResult<Json<Value>> {
    let udid = header(&headers, "x-udid")
        .ok_or_else(|| AppError::BadRequest("chybí hlavička X-UDID".into()))?;
    // UDID jde do názvu souboru — povol jen bezpečné znaky (proti path traversal).
    if udid.is_empty() || !udid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::BadRequest("neplatný UDID".into()));
    }
    let name = header(&headers, "x-name").unwrap_or_else(|| "iPad".to_string());
    let address = header(&headers, "x-address");

    let path = st.cfg.pairing_dir().join(format!("{udid}.plist"));
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| AppError::Other(e.into()))?;

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO device (udid, name, address, pairing_path, created_at, last_seen)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(udid) DO UPDATE SET name = excluded.name, address = excluded.address,
             pairing_path = excluded.pairing_path, last_seen = excluded.last_seen",
    )
    .bind(&udid)
    .bind(&name)
    .bind(&address)
    .bind(path.to_string_lossy().to_string())
    .bind(&now)
    .bind(&now)
    .execute(&st.db)
    .await?;

    Ok(Json(json!({ "ok": true, "udid": udid })))
}

/// DEBUG: nainstaluje už podepsanou IPA ({ipa_id}-signed.ipa) přímo na zařízení,
/// bez resignu/auth — pro test RemoteXPC tunelu nezávisle na Apple throttlu.
pub async fn debug_install_direct(
    State(st): State<AppState>,
    Json(req): Json<InstallRequest>,
) -> AppResult<Json<Value>> {
    let dev = sqlx::query_as::<_, Device>("SELECT * FROM device WHERE udid = ?")
        .bind(&req.device_udid)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| AppError::NotFound("zařízení".into()))?;
    let signed = st.cfg.ipa_dir().join(format!("{}-signed.ipa", req.ipa_id));
    if !signed.exists() {
        return Err(AppError::BadRequest(format!("chybí {}", signed.display())));
    }
    crate::device::install_signed(&dev, &signed, |_, _| async {})
        .await
        .map_err(AppError::Other)?;
    Ok(Json(json!({ "ok": true })))
}

/// Přehled App ID účtu z Developer Services (globální — i AltStore/Xcode).
pub async fn account_appids(State(st): State<AppState>) -> AppResult<Json<Value>> {
    let (team_id, ids) = st.apple.account_app_ids().await.map_err(AppError::Other)?;
    Ok(Json(json!({
        "team_id": team_id,
        "count": ids.len(),
        "max": 10,
        "app_ids": ids,
    })))
}

// ---------------------------------------------------- párování (USB, v serveru)

/// Seznam UDID zařízení připojených přes USB.
pub async fn pair_usb_list() -> AppResult<Json<Vec<String>>> {
    let list = crate::pairing::list_usb().await.map_err(AppError::Other)?;
    Ok(Json(list))
}

#[derive(serde::Deserialize)]
pub struct PairUsbRequest {
    pub udid: Option<String>,
    /// Ruční IP (přebije auto-detekci, když je zadaná).
    pub address: Option<String>,
}

/// Spáruje připojený iPad přes USB, uloží pairing file + zařízení do DB a zkusí
/// automaticky zjistit jeho IP ve Wi-Fi (Bonjour). Bez CLI — vše v serveru.
pub async fn pair_usb(
    State(st): State<AppState>,
    Json(req): Json<PairUsbRequest>,
) -> AppResult<Json<Value>> {
    let (res, serialized) = crate::pairing::pair_usb(req.udid.as_deref())
        .await
        .map_err(AppError::Other)?;

    let path = st.cfg.pairing_dir().join(format!("{}.plist", res.udid));
    tokio::fs::write(&path, &serialized)
        .await
        .map_err(|e| AppError::Other(e.into()))?;

    // Ruční adresa má přednost před auto-detekcí.
    let address = req.address.clone().filter(|s| !s.is_empty()).or_else(|| res.address.clone());

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO device (udid, name, address, pairing_path, ios_version, created_at, last_seen)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(udid) DO UPDATE SET name = excluded.name, address = excluded.address,
             pairing_path = excluded.pairing_path, ios_version = excluded.ios_version,
             last_seen = excluded.last_seen",
    )
    .bind(&res.udid)
    .bind(&res.name)
    .bind(&address)
    .bind(path.to_string_lossy().to_string())
    .bind(&res.ios_version)
    .bind(&now)
    .bind(&now)
    .execute(&st.db)
    .await?;

    Ok(Json(json!({
        "udid": res.udid,
        "name": res.name,
        "ios_version": res.ios_version,
        "address": address,
        "auto_detected_ip": res.address,
        "wifi_mac": res.wifi_mac,
    })))
}

#[derive(serde::Deserialize)]
pub struct SetAddressRequest {
    pub address: String,
}

/// Znovu automaticky zjistí IP zařízení (usbmuxd) a uloží ji.
pub async fn detect_device_ip(
    State(st): State<AppState>,
    Path(udid): Path<String>,
) -> AppResult<Json<Value>> {
    let ip = crate::pairing::detect_ip(&udid).await;
    if let Some(addr) = &ip {
        sqlx::query("UPDATE device SET address = ? WHERE udid = ?")
            .bind(addr)
            .bind(&udid)
            .execute(&st.db)
            .await?;
    }
    Ok(Json(json!({ "address": ip })))
}

/// Ruční nastavení IP zařízení (fallback, když auto-detekce selže).
pub async fn set_device_address(
    State(st): State<AppState>,
    Path(udid): Path<String>,
    Json(req): Json<SetAddressRequest>,
) -> AppResult<Json<Value>> {
    let res = sqlx::query("UPDATE device SET address = ? WHERE udid = ?")
        .bind(&req.address)
        .bind(&udid)
        .execute(&st.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("zařízení".into()));
    }
    Ok(Json(json!({ "ok": true, "address": req.address })))
}

// ------------------------------------------------------------- IPA katalog

pub async fn list_ipa(State(st): State<AppState>) -> AppResult<Json<Vec<Ipa>>> {
    let items = sqlx::query_as::<_, Ipa>("SELECT * FROM ipa ORDER BY created_at DESC")
        .fetch_all(&st.db)
        .await?;
    Ok(Json(items))
}

pub async fn upload_ipa(
    State(st): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<Ipa>> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            let filename = field
                .file_name()
                .unwrap_or("app.ipa")
                .to_string();
            let data = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            let ipa = ipautil::store_uploaded(&st, &filename, &data).await?;
            return Ok(Json(ipa));
        }
    }
    Err(AppError::BadRequest("chybí pole 'file'".into()))
}

pub async fn delete_ipa(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT path FROM ipa WHERE id = ?")
        .bind(&id)
        .fetch_optional(&st.db)
        .await?;
    if let Some((path,)) = row {
        let _ = tokio::fs::remove_file(path).await;
    }
    sqlx::query("DELETE FROM ipa WHERE id = ?")
        .bind(&id)
        .execute(&st.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

// ------------------------------------------------------------- instalace

pub async fn install(
    State(st): State<AppState>,
    Json(req): Json<InstallRequest>,
) -> AppResult<Json<Job>> {
    // Ověř, že zařízení i IPA existují.
    let _dev = sqlx::query_as::<_, Device>("SELECT * FROM device WHERE udid = ?")
        .bind(&req.device_udid)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| AppError::NotFound("zařízení".into()))?;
    let _ipa = sqlx::query_as::<_, Ipa>("SELECT * FROM ipa WHERE id = ?")
        .bind(&req.ipa_id)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| AppError::NotFound("IPA".into()))?;

    let job = jobs::enqueue_install(&st, &req.device_udid, &req.ipa_id).await?;
    jobs::spawn_worker(st.clone(), job.id);
    Ok(Json(job))
}

pub async fn list_installations(
    State(st): State<AppState>,
) -> AppResult<Json<Vec<Installation>>> {
    let items =
        sqlx::query_as::<_, Installation>("SELECT * FROM installation ORDER BY id DESC")
            .fetch_all(&st.db)
            .await?;
    Ok(Json(items))
}

/// Vrací PNG ikonu IPA (uloženou při uploadu).
pub async fn icon(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl axum::response::IntoResponse> {
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT icon_path FROM ipa WHERE id = ?")
        .bind(&id)
        .fetch_optional(&st.db)
        .await?;
    let path = row
        .and_then(|r| r.0)
        .ok_or_else(|| AppError::NotFound("ikona".into()))?;
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| AppError::NotFound("ikona".into()))?;
    Ok(([(axum::http::header::CONTENT_TYPE, "image/png")], data))
}

// ------------------------------------------------------------- úlohy

pub async fn list_jobs(State(st): State<AppState>) -> AppResult<Json<Vec<Job>>> {
    let items =
        sqlx::query_as::<_, Job>("SELECT * FROM job ORDER BY id DESC LIMIT 50")
            .fetch_all(&st.db)
            .await?;
    Ok(Json(items))
}

/// Zruší běžící úlohu.
pub async fn cancel_job(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let cancelled = crate::jobs::cancel(&st, id).await.map_err(AppError::Other)?;
    Ok(Json(json!({ "cancelled": cancelled })))
}

// ------------------------------------------------------------- util

fn header(h: &axum::http::HeaderMap, name: &str) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
}
