//! Refresh scheduler — jádro celého projektu.
//!
//! Běží jako tokio úloha, každou hodinu projde instalace a ty, kterým do expirace
//! profilu zbývá méně než `refresh_before_days`, přepodepíše a přeinstaluje.
//! Refresh iniciuje SERVER (ne iOS appka), takže iOS nemá jak ho zaškrtit — appka
//! na iPadu musí jen být dosažitelná na síti.

use std::time::Duration;

use chrono::Utc;

use crate::models::{Device, Installation, Ipa};
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
        tracing::info!(
            "refresh: instalace {} (device {}) vyprší {:?}",
            inst.id, inst.device_udid, inst.profile_expires
        );
        if let Err(e) = refresh_one(st, &inst).await {
            tracing::warn!("refresh instalace {} selhal: {e:?}", inst.id);
            let _ = sqlx::query("UPDATE installation SET status='error', error=? WHERE id=?")
                .bind(e.to_string())
                .bind(inst.id)
                .execute(&st.db)
                .await;
        }
    }
    Ok(())
}

async fn refresh_one(st: &AppState, inst: &Installation) -> anyhow::Result<()> {
    let device = sqlx::query_as::<_, Device>("SELECT * FROM device WHERE udid = ?")
        .bind(&inst.device_udid)
        .fetch_one(&st.db)
        .await?;
    let ipa = sqlx::query_as::<_, Ipa>("SELECT * FROM ipa WHERE id = ?")
        .bind(&inst.ipa_id)
        .fetch_one(&st.db)
        .await?;

    // Nejdřív rychlý ping — když je iPad mimo síť, zkusíme příště (bez chyby).
    if let Err(e) = crate::device::ping(&device).await {
        anyhow::bail!("zařízení nedosažitelné, zkusím později: {e}");
    }

    let expires = crate::pipeline::install_flow(st, &device, &ipa, |_p, _m| async {}).await?;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE installation SET profile_expires=?, last_installed=?, status='installed', error=NULL WHERE id=?",
    )
    .bind(expires.map(|d| d.to_rfc3339()))
    .bind(&now)
    .bind(inst.id)
    .execute(&st.db)
    .await?;
    tracing::info!("refresh instalace {} hotov", inst.id);
    Ok(())
}
