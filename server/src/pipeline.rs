//! Orchestration of the whole install flow: sign → transfer → install.
//! This is where the `apple` (account, profile), `signer` (signing) and `device`
//! (tunnel + installation_proxy) modules come together.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::models::{Device, Ipa};
use crate::state::AppState;

/// Runs the whole flow for one IPA on one device.
/// Returns the provisioning profile's expiration (for the refresh scheduler).
pub async fn install_flow<F, Fut>(
    st: &AppState,
    device: &Device,
    ipa: &Ipa,
    progress: F,
) -> anyhow::Result<Option<DateTime<Utc>>>
where
    F: Fn(u64, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    progress(5, "připravuji podpis".into()).await;

    // 1) Sign the IPA under our App ID (rewrites the bundle id, inserts the profile,
    //    signs the main binary as well as nested frameworks/extensions).
    let signed = crate::signer::resign(st, ipa, device)
        .await
        .map_err(|e| anyhow::anyhow!("podpis selhal: {e:#}"))?;

    progress(45, "podepsáno, navazuji spojení se zařízením".into()).await;

    // 2) Install via idevice (iOS 17+ → RemoteXPC tunnel + RSD).
    let dev_progress = progress.clone();
    crate::device::install_signed(
        device,
        &signed.path,
        move |pct, msg| {
            let cb = dev_progress.clone();
            async move {
                // We map the installation to 45–100%.
                let mapped = 45 + (pct * 55 / 100);
                cb(mapped, msg).await;
            }
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("instalace selhala: {e}"))?;

    progress(100, "hotovo".into()).await;
    Ok(signed.profile_expires)
}
