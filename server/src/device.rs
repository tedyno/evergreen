//! Komunikace se zařízením přes idevice.
//!
//! iOS 17+ vyžaduje RemoteXPC: navážeme CoreDeviceProxy tunel a přes *userspace*
//! software tunel (žádný TUN/NET_ADMIN nepotřeba) uděláme RSD handshake a pustíme
//! installation_proxy nad RSD transportem.
//!
//! POZOR: reálná instalace se dá ověřit jen s připojeným zařízením — tenhle modul
//! je proto plně testovatelný až ráno s iPadem. Struktura odpovídá ověřeným
//! příkladům z idevice/tools (app_service.rs, ideviceinstaller.rs).

use std::future::Future;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;

use idevice::core_device_proxy::CoreDeviceProxy;
use idevice::pairing_file::PairingFile;
use idevice::provider::TcpProvider;
use idevice::rsd::RsdHandshake;
use idevice::utils::installation;
use idevice::IdeviceService;

use crate::models::Device;

/// Sestaví síťový provider ze záznamu zařízení (IP + pairing file).
fn provider_for(device: &Device) -> anyhow::Result<TcpProvider> {
    let addr = device
        .address
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("zařízení nemá známou IP adresu (Wi-Fi)"))?;
    let addr = IpAddr::from_str(addr).map_err(|e| anyhow::anyhow!("neplatná IP: {e}"))?;
    let pairing_file = PairingFile::read_from_file(&device.pairing_path)
        .map_err(|e| anyhow::anyhow!("nelze načíst pairing file: {e:?}"))?;
    Ok(TcpProvider {
        addr,
        scope_id: None,
        pairing_file,
        label: "homesign".to_string(),
    })
}

/// Nainstaluje (nebo upgraduje) už podepsané IPA na zařízení přes RemoteXPC tunel.
pub async fn install_signed<F, Fut>(
    device: &Device,
    signed_ipa: &Path,
    progress: F,
) -> anyhow::Result<()>
where
    F: Fn(u64) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    let provider = provider_for(device)?;

    // 1) CoreDeviceProxy tunel (iOS 17+).
    let proxy = CoreDeviceProxy::connect(&provider)
        .await
        .map_err(|e| anyhow::anyhow!("CoreDeviceProxy: {e:?}"))?;
    let rsd_port = proxy.tunnel_info().server_rsd_port;

    // 2) Userspace software tunel — nepotřebuje TUN device ani root.
    let adapter = proxy
        .create_software_tunnel()
        .map_err(|e| anyhow::anyhow!("software tunnel: {e:?}"))?;
    let mut adapter = adapter.to_async_handle();

    // 3) RSD handshake přes tunel.
    let stream = adapter
        .connect(rsd_port)
        .await
        .map_err(|e| anyhow::anyhow!("RSD connect: {e:?}"))?;
    let mut handshake = RsdHandshake::new(stream)
        .await
        .map_err(|e| anyhow::anyhow!("RSD handshake: {e:?}"))?;

    // 4) installation_proxy nad RSD — upgrade zachová data appky (důležité pro refresh).
    installation::upgrade_package_with_callback_rsd(
        &mut adapter,
        &mut handshake,
        signed_ipa,
        None,
        move |(pct, _): (u64, ())| {
            let cb = progress.clone();
            async move { cb(pct).await }
        },
        (),
    )
    .await
    .map_err(|e| anyhow::anyhow!("installation_proxy: {e:?}"))?;

    Ok(())
}

/// Ověří, že pairing file funguje a zařízení je dosažitelné (health-check).
pub async fn ping(device: &Device) -> anyhow::Result<()> {
    let provider = provider_for(device)?;
    let _proxy = CoreDeviceProxy::connect(&provider)
        .await
        .map_err(|e| anyhow::anyhow!("zařízení nedosažitelné: {e:?}"))?;
    Ok(())
}
