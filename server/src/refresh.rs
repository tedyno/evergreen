//! Refresh scheduler — jádro celého projektu.
//!
//! Běží jako tokio úloha, každou hodinu projde instalace a ty, kterým do expirace
//! profilu zbývá méně než `refresh_before_days`, přepodepíše a přeinstaluje.
//! Refresh iniciuje SERVER (ne iOS appka), takže iOS nemá jak ho zaškrtit — appka
//! na iPadu musí jen být dosažitelná na síti.

use std::time::Duration;

use chrono::Utc;

use crate::models::{Device, Installation};
use crate::state::AppState;

pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        // Malé zpoždění po startu, ať se server stihne rozběhnout.
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
            None => false, // bez známé expirace nerefreshujeme automaticky
        };
        if !needs {
            continue;
        }

        // Rychlý ping — když je iPad mimo síť, zkusíme příště (bez zápisu chyby,
        // ať se resign log nezaplní neúspěchy z toho, že iPad zrovna spí).
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
        // Zařaď jako úlohu (kind 'refresh') → objeví se v resign logu (Úlohy).
        match crate::jobs::enqueue(st, &inst.device_udid, &inst.ipa_id, "refresh").await {
            Ok(job) => crate::jobs::spawn_worker(st.clone(), job.id),
            Err(e) => tracing::warn!("refresh enqueue instalace {} selhal: {e:?}", inst.id),
        }
    }
    Ok(())
}
