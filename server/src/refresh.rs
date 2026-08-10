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
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
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
