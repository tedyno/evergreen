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

    Ok(Json(outcome_json(&outcome)))
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

// ------------------------------------------------------------- util

fn header(h: &axum::http::HeaderMap, name: &str) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
}
