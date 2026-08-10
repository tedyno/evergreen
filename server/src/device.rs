//! Communication with the device via idevice.
//!
//! iOS 17+ requires RemoteXPC: we establish a CoreDeviceProxy tunnel and, over a
//! *userspace* software tunnel (no TUN/NET_ADMIN needed), perform the RSD handshake
//! and run installation_proxy over the RSD transport.
//!
//! NOTE: a real installation can only be verified with a connected device — this
//! module is therefore only fully testable in the morning with the iPad. The
//! structure mirrors the proven examples in idevice/tools (app_service.rs, ideviceinstaller.rs).

use std::future::Future;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;

use idevice::core_device_proxy::CoreDeviceProxy;
use idevice::pairing_file::PairingFile;
use idevice::provider::{IdeviceProvider, TcpProvider};
use idevice::rsd::RsdHandshake;
use idevice::services::afc::{opcode::AfcFopenMode, AfcClient};
use idevice::services::installation_proxy::InstallationProxyClient;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::{IdeviceService, RsdService};
use tokio::io::AsyncReadExt;

use crate::models::Device;

const PUBLIC_STAGING: &str = "PublicStaging";
const UPLOAD_CHUNK: usize = 8 * 1024 * 1024; // 8 MB

/// Network provider from the device record (IP + pairing file).
fn tcp_provider(device: &Device) -> anyhow::Result<TcpProvider> {
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

/// Picks the best available transport: prefer USB (on iOS 17+ `CoreDeviceProxy`
/// works over usbmux), otherwise fall back to Wi-Fi (RemoteXPC).
async fn best_provider(device: &Device) -> anyhow::Result<Box<dyn IdeviceProvider>> {
    // USB, when the iPad is connected by cable.
    if let Ok(mut u) = UsbmuxdConnection::default().await {
        if let Ok(devs) = u.get_devices().await {
            if let Some(d) = devs
                .into_iter()
                .find(|d| d.udid == device.udid && d.connection_type == Connection::Usb)
            {
                tracing::info!("instalace přes USB ({})", device.udid);
                return Ok(Box::new(d.to_provider(UsbmuxdAddr::default(), "homesign-install")));
            }
        }
    }
    // Otherwise Wi-Fi.
    tracing::info!("instalace přes Wi-Fi ({:?})", device.address);
    Ok(Box::new(tcp_provider(device)?))
}

/// Installs (or upgrades) an already-signed IPA on the device over the RemoteXPC tunnel.
/// The callback receives (percentage 0–100 within the install phase, a text message with MB).
pub async fn install_signed<F, Fut>(
    device: &Device,
    signed_ipa: &Path,
    progress: F,
) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    let bundle_id = read_ipa_bundle_id(signed_ipa).await?;
    let provider = best_provider(device).await?;

    // Direct AFC over usbmux (without the software tunnel — it stalls on bulk transfer).
    // AFC is a classic service reachable via lockdown, even on iOS 17+ over USB.
    let total = tokio::fs::metadata(signed_ipa).await?.len();
    let total_mb = total / (1024 * 1024);
    let remote_path = format!("{PUBLIC_STAGING}/homesign.ipa");
    {
        tracing::info!("AFC connect (přímý přes usbmux)…");
        let mut afc = AfcClient::connect(provider.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("AFC: {e:?}"))?;
        tracing::info!("AFC OK; mk_dir PublicStaging…");
        let _ = afc.mk_dir(PUBLIC_STAGING).await;

        let mut file = tokio::fs::File::open(signed_ipa).await?;
        let mut fd = afc
            .open(remote_path.clone(), AfcFopenMode::WrOnly)
            .await
            .map_err(|e| anyhow::anyhow!("AFC open: {e:?}"))?;
        tracing::info!("nahrávám…");

        let mut buf = vec![0u8; UPLOAD_CHUNK];
        let mut sent: u64 = 0;
        let mut last_report: u64 = 0;
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            // write_entire = the native AFC write (sends packets and reads responses),
            // which actively pumps the tunnel — unlike AsyncWrite, which stalls.
            fd.write_entire(&buf[..n])
                .await
                .map_err(|e| anyhow::anyhow!("AFC write: {e:?}"))?;
            sent += n as u64;
            // Report roughly every 4 MB. Upload = 0–85% of the install phase.
            if sent - last_report >= 4 * 1024 * 1024 || sent == total {
                last_report = sent;
                let pct = if total > 0 { sent * 85 / total } else { 0 };
                let sent_mb = sent / (1024 * 1024);
                tracing::info!("upload {sent_mb}/{total_mb} MB");
                progress(pct, format!("Nahrávám {sent_mb} / {total_mb} MB")).await;
            }
        }
    }

    // InstallationProxy Upgrade (preserves the app's data) — classically over usbmux.
    tracing::info!("installation_proxy connect (přímý)…");
    let mut inst = InstallationProxyClient::connect(provider.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("InstallationProxy: {e:?}"))?;
    let mut opt = plist::Dictionary::new();
    opt.insert("CFBundleIdentifier".into(), plist::Value::String(bundle_id.clone()));
    let options = plist::Value::Dictionary(opt);
    let cb = progress.clone();
    inst.upgrade_with_callback(
        remote_path,
        Some(options),
        move |(pct, _): (u64, ())| {
            let cb = cb.clone();
            async move {
                // Installation = 85–100% of the install phase.
                let mapped = 85 + pct * 15 / 100;
                cb(mapped, format!("Instaluji na iPad… {pct} %")).await;
            }
        },
        (),
    )
    .await
    .map_err(|e| anyhow::anyhow!("installation_proxy: {e:?}"))?;

    Ok(())
}

/// Reads CFBundleIdentifier from `Payload/*.app/Info.plist` in the IPA (just the
/// small entry, not the whole file).
async fn read_ipa_bundle_id(ipa: &Path) -> anyhow::Result<String> {
    let path = ipa.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let f = std::fs::File::open(&path)?;
        let mut zip = zip::ZipArchive::new(f)?;
        // Find the root Payload/<App>.app/Info.plist.
        let mut name: Option<String> = None;
        for i in 0..zip.len() {
            let n = zip.by_index(i)?.name().to_string();
            let parts: Vec<&str> = n.split('/').collect();
            if parts.len() == 3 && parts[0] == "Payload" && parts[1].ends_with(".app") && parts[2] == "Info.plist" {
                name = Some(n);
                break;
            }
        }
        let name = name.ok_or_else(|| anyhow::anyhow!("Info.plist nenalezen"))?;
        let mut buf = Vec::new();
        zip.by_name(&name)?.read_to_end(&mut buf)?;
        let val: plist::Value = plist::from_bytes(&buf)?;
        val.as_dictionary()
            .and_then(|d| d.get("CFBundleIdentifier"))
            .and_then(|v| v.as_string())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("chybí CFBundleIdentifier"))
    })
    .await?
}

/// Verifies that the pairing file works and the device is reachable (health-check).
pub async fn ping(device: &Device) -> anyhow::Result<()> {
    let provider = best_provider(device).await?;
    let _proxy = CoreDeviceProxy::connect(provider.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("zařízení nedosažitelné: {e:?}"))?;
    Ok(())
}
