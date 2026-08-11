//! Communication with the device via idevice (USB) and `devicectl` (Wi-Fi).
//!
//! Two install transports:
//!  * **USB** (preferred): direct AFC + installation_proxy over usbmux — fast,
//!    no extra tooling.
//!  * **Wi-Fi** (iOS 17+): we shell out to Apple's `devicectl` (CoreDevice),
//!    which already maintains the encrypted tunnel to the paired device. This
//!    needs Xcode / Command Line Tools installed, but requires no custom tunnel
//!    and no suspending of system daemons.

use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;

use idevice::core_device_proxy::CoreDeviceProxy;
use idevice::pairing_file::PairingFile;
use idevice::provider::{IdeviceProvider, TcpProvider};
use idevice::services::afc::{opcode::AfcFopenMode, AfcClient};
use idevice::services::installation_proxy::InstallationProxyClient;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::IdeviceService;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::models::Device;

const PUBLIC_STAGING: &str = "PublicStaging";
const REMOTE_IPA: &str = "PublicStaging/evergreen.ipa";
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
        label: "evergreen".to_string(),
    })
}

/// Returns a USB provider for this device when it is connected by cable.
async fn usb_provider(device: &Device) -> Option<Box<dyn IdeviceProvider>> {
    let mut u = UsbmuxdConnection::default().await.ok()?;
    let devs = u.get_devices().await.ok()?;
    let d = devs
        .into_iter()
        .find(|d| d.udid == device.udid && d.connection_type == Connection::Usb)?;
    Some(Box::new(d.to_provider(UsbmuxdAddr::default(), "evergreen-install")))
}

/// Whether `devicectl` (Apple CoreDevice) is available — required for Wi-Fi installs.
pub fn devicectl_available() -> bool {
    std::process::Command::new("/usr/bin/xcrun")
        .args(["--find", "devicectl"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True if the device is reachable over a USB cable right now (USB install works even
/// when the screen is locked, so it's always preferred).
pub async fn has_usb(device: &Device) -> bool {
    usb_provider(device).await.is_some()
}

/// Current lock state via `devicectl device info lockState` (Some(true) = locked now,
/// Some(false) = unlocked). None if it can't be determined (no Xcode / unreachable).
/// This lightweight query works over the CoreDevice tunnel even while the device is
/// locked — unlike a real install, which iOS refuses on a locked device.
pub async fn is_locked(device: &Device) -> Option<bool> {
    if !devicectl_available() {
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("evergreen-lock-{}.json", device.udid));
    let out = tokio::process::Command::new("/usr/bin/xcrun")
        .args(["devicectl", "device", "info", "lockState", "--device"])
        .arg(&device.udid)
        .arg("--json-output")
        .arg(&tmp)
        .output()
        .await
        .ok()?;
    let locked = if out.status.success() {
        tokio::fs::read(&tmp)
            .await
            .ok()
            .and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok())
            .and_then(|v| v.get("result")?.get("passcodeRequired")?.as_bool())
    } else {
        None
    };
    let _ = tokio::fs::remove_file(&tmp).await;
    locked
}

/// Installs (or upgrades) an already-signed IPA on the device.
/// The callback receives (percentage 0–100 within the install phase, a text message).
pub async fn install_signed<F, Fut>(
    device: &Device,
    signed_ipa: &Path,
    progress: F,
) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    // USB: direct AFC + installation_proxy (fast).
    if let Some(provider) = usb_provider(device).await {
        tracing::info!("instalace přes USB ({})", device.udid);
        let bundle_id = read_ipa_bundle_id(signed_ipa).await?;
        let mut afc = AfcClient::connect(provider.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("AFC: {e:?}"))?;
        upload_ipa(&mut afc, signed_ipa, &progress).await?;
        let mut inst = InstallationProxyClient::connect(provider.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("InstallationProxy: {e:?}"))?;
        return upgrade(&mut inst, &bundle_id, &progress).await;
    }

    // Wi-Fi: hand off to Apple's devicectl over the CoreDevice tunnel.
    tracing::info!("instalace přes Wi-Fi / devicectl ({})", device.udid);
    install_via_devicectl(device, signed_ipa, &progress).await
}

/// Uploads the IPA into the AFC media jail at `PublicStaging/homesign.ipa`.
/// Upload = 0–85% of the phase.
async fn upload_ipa<F, Fut>(afc: &mut AfcClient, signed_ipa: &Path, progress: &F) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut,
    Fut: Future<Output = ()>,
{
    let total = tokio::fs::metadata(signed_ipa).await?.len();
    let total_mb = total / (1024 * 1024);
    let _ = afc.mk_dir(PUBLIC_STAGING).await;

    let mut file = tokio::fs::File::open(signed_ipa).await?;
    let mut fd = afc
        .open(REMOTE_IPA.to_string(), AfcFopenMode::WrOnly)
        .await
        .map_err(|e| anyhow::anyhow!("AFC open: {e:?}"))?;

    let mut buf = vec![0u8; UPLOAD_CHUNK];
    let mut sent: u64 = 0;
    let mut last_report: u64 = 0;
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        fd.write_entire(&buf[..n])
            .await
            .map_err(|e| anyhow::anyhow!("AFC write: {e:?}"))?;
        sent += n as u64;
        if sent - last_report >= 4 * 1024 * 1024 || sent == total {
            last_report = sent;
            let pct = if total > 0 { sent * 85 / total } else { 0 };
            let sent_mb = sent / (1024 * 1024);
            tracing::info!("upload {sent_mb}/{total_mb} MB");
            progress(pct, format!("Nahrávám {sent_mb} / {total_mb} MB")).await;
        }
    }
    // Must close explicitly: idevice finalizes the file here and panics on drop otherwise.
    fd.close()
        .await
        .map_err(|e| anyhow::anyhow!("AFC close: {e:?}"))?;
    Ok(())
}

/// Runs installation_proxy Upgrade (preserves the app's data). Install = 85–100%.
async fn upgrade<F, Fut>(
    inst: &mut InstallationProxyClient,
    bundle_id: &str,
    progress: &F,
) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    let mut opt = plist::Dictionary::new();
    opt.insert("CFBundleIdentifier".into(), plist::Value::String(bundle_id.to_string()));
    let options = plist::Value::Dictionary(opt);
    let cb = progress.clone();
    inst.upgrade_with_callback(
        REMOTE_IPA.to_string(),
        Some(options),
        move |(pct, _): (u64, ())| {
            let cb = cb.clone();
            async move {
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

/// Wireless install via `xcrun devicectl device install app`, which uses Apple's
/// own CoreDevice tunnel to the paired device (no custom tunnel, no daemon fiddling).
async fn install_via_devicectl<F, Fut>(
    device: &Device,
    signed_ipa: &Path,
    progress: &F,
) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut,
    Fut: Future<Output = ()>,
{
    if !devicectl_available() {
        anyhow::bail!(
            "bezdrátová instalace vyžaduje Xcode / Command Line Tools (devicectl) — \
             připoj iPad USB kabelem, nebo nainstaluj Xcode"
        );
    }

    // iOS refuses a network install on a locked device, so fail fast with a clear
    // message instead of a long crawl that ends in a cryptic "Connection invalid".
    if is_locked(device).await == Some(true) {
        anyhow::bail!(
            "iPad je zamčený — bezdrátová instalace vyžaduje odemčený iPad. \
             Odemkni ho a zkus znovu (nebo připoj USB kabel, ten funguje i zamčený)."
        );
    }

    progress(5, "Rozbaluji balíček…".into()).await;
    let (workdir, app_dir) = extract_app(signed_ipa).await?;

    progress(20, "Instaluji přes Wi-Fi (devicectl)…".into()).await;
    let mut child = tokio::process::Command::new("/usr/bin/xcrun")
        .args(["devicectl", "device", "install", "app", "--device"])
        .arg(&device.udid)
        .arg(&app_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("nelze spustit devicectl: {e}"))?;

    // Drain stderr concurrently so a full pipe can't deadlock us.
    let mut stderr_pipe = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = p.read_to_string(&mut s).await;
        }
        s
    });

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::info!("devicectl: {}", line.trim());
            let l = line.to_lowercase();
            // Map devicectl's phase markers onto a coarse progress bar.
            let step = if l.contains("tunnel connection") {
                Some((30, "Navazuji spojení s iPadem…"))
            } else if l.contains("developer disk") {
                Some((45, "Připravuji zařízení…"))
            } else if l.contains("usage assertion") {
                Some((65, "Přenáším a instaluji…"))
            } else if l.contains("app installed") {
                Some((98, "Dokončuji…"))
            } else {
                None
            };
            if let Some((pct, msg)) = step {
                progress(pct, msg.into()).await;
            }
        }
    }

    let status = child.wait().await.map_err(|e| anyhow::anyhow!("devicectl: {e}"))?;
    let stderr = stderr_task.await.unwrap_or_default();
    let _ = tokio::fs::remove_dir_all(&workdir).await;

    if !status.success() {
        let tail: String = stderr.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
        // A dropped connection mid-transfer usually means the iPad locked or left Wi-Fi.
        let hint = if tail.contains("Connection invalid") || tail.contains("unexpectedly closed") {
            " — iPad se nejspíš během instalace zamkl nebo vypadl z Wi-Fi; nech ho odemčený a na síti"
        } else {
            ""
        };
        anyhow::bail!("devicectl selhal ({status}): {tail}{hint}");
    }
    progress(100, "Hotovo".into()).await;
    Ok(())
}

/// Unzips a signed IPA into a temp work dir and returns (workdir, `Payload/*.app`).
async fn extract_app(signed_ipa: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let stem = signed_ipa
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ipa");
    let workdir = std::env::temp_dir().join(format!("evergreen-install-{stem}"));
    let _ = tokio::fs::remove_dir_all(&workdir).await;
    tokio::fs::create_dir_all(&workdir).await?;

    let out = tokio::process::Command::new("/usr/bin/unzip")
        .args(["-q", "-o"])
        .arg(signed_ipa)
        .arg("-d")
        .arg(&workdir)
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!("rozbalení IPA selhalo: {}", String::from_utf8_lossy(&out.stderr));
    }

    let payload = workdir.join("Payload");
    let mut rd = tokio::fs::read_dir(&payload).await?;
    while let Some(entry) = rd.next_entry().await? {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("app") {
            return Ok((workdir, p));
        }
    }
    anyhow::bail!("v IPA nenalezen .app v Payload/");
}

/// Reads CFBundleIdentifier from `Payload/*.app/Info.plist` in the IPA.
async fn read_ipa_bundle_id(ipa: &Path) -> anyhow::Result<String> {
    let path = ipa.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let f = std::fs::File::open(&path)?;
        let mut zip = zip::ZipArchive::new(f)?;
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

/// Verifies that the device is reachable (health-check). USB uses CoreDeviceProxy;
/// otherwise we just confirm we can build a Wi-Fi provider.
pub async fn ping(device: &Device) -> anyhow::Result<()> {
    if let Some(provider) = usb_provider(device).await {
        let _proxy = CoreDeviceProxy::connect(provider.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("zařízení nedosažitelné: {e:?}"))?;
        return Ok(());
    }
    let _p = tcp_provider(device)?;
    Ok(())
}
