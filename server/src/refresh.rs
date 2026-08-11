//! Refresh scheduler — the core of the whole project.
//!
//! Runs as a tokio task; every hour it goes through the installations and re-signs
//! and re-installs those whose profile has less than `refresh_before_days` until expiration.
//! The refresh is initiated by the SERVER (not the iOS app), so iOS has no way to block it —
//! the app on the iPad only needs to be reachable on the network.

use std::time::Duration;

use chrono::Utc;

use crate::models::{Device, Installation};
use crate::state::AppState;

pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        // A small delay after startup so the server has time to come up.
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            if let Err(e) = tick(&st).await {
                tracing::error!("refresh tick selhal: {e:?}");
            }
            // While something is waiting for the iPad to be unlocked, re-check every 30 s so
            // the renewal fires soon after unlock; otherwise the normal hourly cadence.
            let delay = if any_blocked(&st).await { 30 } else { 3600 };
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    });
}

/// Runs one refresh pass immediately (used by the "I unlocked it" button).
pub async fn run_once(st: &AppState) -> anyhow::Result<()> {
    tick(st).await
}

/// Whether any renewal is currently waiting for the iPad to be unlocked.
async fn any_blocked(st: &AppState) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM job WHERE status = 'blocked'")
        .fetch_one(&st.db)
        .await
        .map(|n| n > 0)
        .unwrap_or(false)
}

async fn tick(st: &AppState) -> anyhow::Result<()> {
    let threshold = Utc::now() + chrono::Duration::days(st.cfg.refresh_before_days);

    let installs = sqlx::query_as::<_, Installation>(
        "SELECT * FROM installation WHERE status IN ('installed','expired')",
    )
    .fetch_all(&st.db)
    .await?;

    for inst in installs {
        let needs = match inst.expires_at() {
            Some(exp) => exp <= threshold,
            None => false, // without a known expiration we do not refresh automatically
        };
        if !needs {
            // Clear any stale "waiting for unlock" marker — this app doesn't need renewing.
            let _ = sqlx::query(
                "DELETE FROM job WHERE device_udid = ? AND ipa_id = ? AND kind = 'refresh' AND status = 'blocked'",
            )
            .bind(&inst.device_udid)
            .bind(&inst.ipa_id)
            .execute(&st.db)
            .await;
            continue;
        }

        // Quick ping — if the iPad is off the network, we try again next time (without
        // logging an error, so the resign log doesn't fill up with failures just because
        // the iPad happens to be asleep).
        let device = match sqlx::query_as::<_, Device>("SELECT * FROM device WHERE udid = ?")
            .bind(&inst.device_udid)
            .fetch_optional(&st.db)
            .await?
        {
            Some(d) => d,
            None => continue,
        };
        if crate::device::ping(&device).await.is_err() {
            tracing::debug!("refresh: zařízení {} mimo síť, zkusím později", inst.device_udid);
            continue;
        }

        // USB works even when the iPad is locked; over Wi-Fi, iOS refuses to install on a
        // locked device — so only proceed wirelessly once we can confirm it's unlocked.
        if !crate::device::has_usb(&device).await {
            match crate::device::is_locked(&device).await {
                Some(false) => {
                    // Was it waiting on a lock? Clear that so the fast polling stops.
                    let _ = sqlx::query(
                        "DELETE FROM job WHERE device_udid = ? AND ipa_id = ? AND kind = 'refresh' AND status = 'blocked'",
                    )
                    .bind(&inst.device_udid)
                    .bind(&inst.ipa_id)
                    .execute(&st.db)
                    .await;
                }
                Some(true) => {
                    // Locked: record it once (not every hour) so the app can nudge the
                    // user to unlock. It renews on the next unlock.
                    let latest: Option<String> = sqlx::query_scalar(
                        "SELECT status FROM job WHERE device_udid = ? AND ipa_id = ? AND kind = 'refresh'
                         ORDER BY id DESC LIMIT 1",
                    )
                    .bind(&inst.device_udid)
                    .bind(&inst.ipa_id)
                    .fetch_optional(&st.db)
                    .await?;
                    if latest.as_deref() != Some("blocked") {
                        tracing::info!("refresh: iPad {} zamčený — čekám na odemčení", inst.device_udid);
                        let _ = crate::jobs::enqueue_blocked(
                            st, &inst.device_udid, &inst.ipa_id,
                            "iPad je zamčený — odemkni ho pro obnovu",
                        )
                        .await;
                    }
                    continue;
                }
                None => {
                    // Unreachable / can't tell — retry later, quietly.
                    tracing::debug!("refresh: iPad {} nedosažitelný, zkusím později", inst.device_udid);
                    continue;
                }
            }
        }

        tracing::info!(
            "refresh: instalace {} vyprší {:?} — zařazuji resign",
            inst.id, inst.profile_expires
        );
        // Enqueue as a job (kind 'refresh') → it shows up in the resign log (Jobs).
        match crate::jobs::enqueue(st, &inst.device_udid, &inst.ipa_id, "refresh").await {
            Ok(job) => crate::jobs::spawn_worker(st.clone(), job.id),
            Err(e) => tracing::warn!("refresh enqueue instalace {} selhal: {e:?}", inst.id),
        }
    }
    Ok(())
}
