//! Orchestrace celého instalačního toku: podpis → přenos → instalace.
//! Sem se sbíhají moduly `apple` (účet, profil), `signer` (podpis) a `device`
//! (tunel + installation_proxy).

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::models::{Device, Ipa};
use crate::state::AppState;

/// Provede celý tok pro jednu IPA na jedno zařízení.
/// Vrací expiraci provisioning profilu (pro scheduler refreshe).
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

    // 1) Podepiš IPA pod naším App ID (přepíše bundle id, vloží profil, podepíše
    //    hlavní binárku i nested frameworky/extensions).
    let signed = crate::signer::resign(st, ipa)
        .await
        .map_err(|e| anyhow::anyhow!("podpis selhal: {e}"))?;

    progress(45, "podepsáno, navazuji spojení se zařízením".into()).await;

    // 2) Nainstaluj přes idevice (iOS 17+ → RemoteXPC tunel + RSD).
    let dev_progress = progress.clone();
    crate::device::install_signed(
        device,
        &signed.path,
        move |pct| {
            let cb = dev_progress.clone();
            async move {
                // Instalace mapujeme na 45–100 %.
                let mapped = 45 + (pct * 55 / 100);
                cb(mapped, format!("instalace {pct}%")).await;
            }
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("instalace selhala: {e}"))?;

    progress(100, "hotovo".into()).await;
    Ok(signed.profile_expires)
}
