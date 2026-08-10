//! Job queue and worker. Install/refresh run asynchronously; the state is streamed
//! into the `job` table so the web UI can poll it and it survives a server restart.

use crate::error::AppResult;
use crate::models::{Device, Ipa, Job};
use crate::state::AppState;

pub async fn enqueue_install(st: &AppState, device_udid: &str, ipa_id: &str) -> AppResult<Job> {
    enqueue(st, device_udid, ipa_id, "install").await
}

/// Enqueues a job of the given kind ('install' | 'refresh') — both are resign+install.
pub async fn enqueue(st: &AppState, device_udid: &str, ipa_id: &str, kind: &str) -> AppResult<Job> {
    let now = chrono::Utc::now().to_rfc3339();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO job (kind, device_udid, ipa_id, status, created_at, updated_at)
         VALUES (?, ?, ?, 'queued', ?, ?) RETURNING id",
    )
    .bind(kind)
    .bind(device_udid)
    .bind(ipa_id)
    .bind(&now)
    .bind(&now)
    .fetch_one(&st.db)
    .await?;

    let job = sqlx::query_as::<_, Job>("SELECT * FROM job WHERE id = ?")
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    Ok(job)
}

/// Starts a worker for the given job in the background (fire-and-forget tokio task).
pub fn spawn_worker(st: AppState, job_id: i64) {
    let jobs = st.running_jobs.clone();
    let task = tokio::spawn(async move {
        if let Err(e) = run_job(&st, job_id).await {
            tracing::error!("job {job_id} selhal: {e:?}");
            let _ = set_status(&st, job_id, "error", 0, Some(&e.to_string())).await;
        }
        if let Ok(mut m) = st.running_jobs.lock() {
            m.remove(&job_id);
        }
    });
    // Register the abort handle SYNCHRONOUSLY (std mutex) — before the worker manages to remove.
    if let Ok(mut m) = jobs.lock() {
        m.insert(job_id, task.abort_handle());
    };
}

/// Cancels a running job (abort the tokio task + mark it in the DB).
pub async fn cancel(st: &AppState, job_id: i64) -> anyhow::Result<bool> {
    let handle = st.running_jobs.lock().ok().and_then(|mut m| m.remove(&job_id));
    if let Some(h) = handle {
        h.abort();
        set_status(st, job_id, "error", 0, Some("zrušeno uživatelem")).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn run_job(st: &AppState, job_id: i64) -> anyhow::Result<()> {
    let job = sqlx::query_as::<_, Job>("SELECT * FROM job WHERE id = ?")
        .bind(job_id)
        .fetch_one(&st.db)
        .await?;

    set_status(st, job_id, "running", 0, Some("start")).await?;

    let device = sqlx::query_as::<_, Device>("SELECT * FROM device WHERE udid = ?")
        .bind(job.device_udid.as_deref().unwrap_or_default())
        .fetch_one(&st.db)
        .await?;
    let ipa = sqlx::query_as::<_, Ipa>("SELECT * FROM ipa WHERE id = ?")
        .bind(job.ipa_id.as_deref().unwrap_or_default())
        .fetch_one(&st.db)
        .await?;

    // The progress callback writes into the DB.
    let st2 = st.clone();
    let progress = move |pct: u64, msg: String| {
        let st = st2.clone();
        async move {
            let _ = set_status(&st, job_id, "running", pct as i64, Some(&msg)).await;
        }
    };

    let result = crate::pipeline::install_flow(st, &device, &ipa, progress).await;

    match result {
        Ok(expires) => {
            record_installation(st, &device, &ipa, expires).await?;
            set_status(st, job_id, "done", 100, Some("hotovo")).await?;
        }
        Err(e) => {
            set_status(st, job_id, "error", 0, Some(&e.to_string())).await?;
            return Err(e);
        }
    }
    Ok(())
}

async fn record_installation(
    st: &AppState,
    device: &Device,
    ipa: &Ipa,
    expires: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let expires_s = expires.map(|d| d.to_rfc3339());
    // signed_bundle_id is derived deterministically (see signer); here we store the original.
    sqlx::query(
        "INSERT INTO installation (device_udid, ipa_id, signed_bundle_id, profile_expires, last_installed, status)
         VALUES (?, ?, ?, ?, ?, 'installed')
         ON CONFLICT(device_udid, ipa_id) DO UPDATE SET
             profile_expires = excluded.profile_expires,
             last_installed = excluded.last_installed,
             status = 'installed', error = NULL",
    )
    .bind(&device.udid)
    .bind(&ipa.id)
    .bind(&ipa.bundle_id)
    .bind(&expires_s)
    .bind(&now)
    .execute(&st.db)
    .await?;
    Ok(())
}

pub async fn set_status(
    st: &AppState,
    job_id: i64,
    status: &str,
    progress: i64,
    message: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE job SET status = ?, progress = ?, message = ?, updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(progress)
    .bind(message)
    .bind(&now)
    .bind(job_id)
    .execute(&st.db)
    .await?;
    Ok(())
}
