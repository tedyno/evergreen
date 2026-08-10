//! Communication with the device via idevice.
//!
//! Two install transports:
//!  * **USB** (preferred): direct AFC + installation_proxy over usbmux. This
//!    bypasses the userspace software tunnel entirely, which is why bulk
//!    transfer is fast and reliable.
//!  * **Wi-Fi** (iOS 17+): there is no usbmux to lean on, so everything goes
//!    through the RemoteXPC tunnel — RemotePairing (Ed25519) → TLS-PSK →
//!    a userspace TCP stack (jktcp) → RSD → AFC + installation_proxy. This
//!    requires a RemotePairing record created earlier over USB (see
//!    `pairing::create_rp_pairing`).
//!
//! The structure mirrors the proven idevice examples (`rppairing.rs`,
//! `pair_rsd_ios.rs`, `ideviceinstaller.rs`).

use std::future::Future;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use idevice::core_device_proxy::CoreDeviceProxy;
use idevice::pairing_file::PairingFile;
use idevice::provider::{IdeviceProvider, TcpProvider};
use idevice::remote_pairing::{connect_tls_psk_tunnel_native, RemotePairingClient, RpPairingFile};
use idevice::rsd::RsdHandshake;
use idevice::services::afc::{opcode::AfcFopenMode, AfcClient};
use idevice::services::installation_proxy::InstallationProxyClient;
use idevice::tcp::adapter::Adapter;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::{IdeviceService, RemoteXpcClient};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use crate::models::Device;

const PUBLIC_STAGING: &str = "PublicStaging";
const REMOTE_IPA: &str = "PublicStaging/homesign.ipa";
const UPLOAD_CHUNK: usize = 8 * 1024 * 1024; // 8 MB
const RP_HOST: &str = "homesign";
const TUNNEL_SERVICE: &str = "com.apple.internal.dt.coredevice.untrusted.tunnelservice";

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

/// Returns a usbmux provider for this device, preferring USB (cable) but falling
/// back to a Wi-Fi network connection that usbmuxd exposes when Wi-Fi sync is on.
/// The bool is true for USB. Over usbmux, classic lockdown services (AFC,
/// installation_proxy) are reachable transparently — the same fast path that
/// AltStore uses for wireless refresh.
async fn muxd_provider(device: &Device) -> Option<(Box<dyn IdeviceProvider>, bool)> {
    let mut u = UsbmuxdConnection::default().await.ok()?;
    let devs = u.get_devices().await.ok()?;
    let mine: Vec<_> = devs.into_iter().filter(|d| d.udid == device.udid).collect();
    // Prefer USB, then any (network) connection.
    let usb = mine.iter().find(|d| d.connection_type == Connection::Usb);
    if let Some(d) = usb {
        return Some((Box::new(d.to_provider(UsbmuxdAddr::default(), "homesign-install")), true));
    }
    let net = mine.first()?;
    Some((Box::new(net.to_provider(UsbmuxdAddr::default(), "homesign-install")), false))
}

/// Installs (or upgrades) an already-signed IPA on the device.
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

    // 1) usbmux (USB cable, or Wi-Fi sync network device): direct AFC +
    //    installation_proxy — fast, bypasses the userspace tunnel.
    if let Some((provider, is_usb)) = muxd_provider(device).await {
        tracing::info!("instalace přes usbmux {} ({})", if is_usb { "USB" } else { "Wi-Fi" }, device.udid);
        return install_via_lockdown(provider.as_ref(), signed_ipa, &bundle_id, &progress).await;
    }

    // 2) Classic lockdown over Wi-Fi (the AltStore path): the device advertises
    //    `_apple-mobdev2._tcp` and listens on lockdown port 62078 on its IP.
    if device.address.is_some() {
        if let Ok(provider) = tcp_provider(device) {
            tracing::info!("instalace přes Wi-Fi lockdown/TCP ({})", device.udid);
            match install_via_lockdown(&provider, signed_ipa, &bundle_id, &progress).await {
                Ok(()) => return Ok(()),
                Err(e) => tracing::warn!("Wi-Fi lockdown selhalo, zkusím RemoteXPC tunel: {e:?}"),
            }
        }
    }

    // 3) Last resort: RemoteXPC tunnel (iOS 17+, needs _remoted._tcp discovery).
    tracing::info!("instalace přes Wi-Fi tunel ({})", device.udid);
    install_over_tunnel(device, signed_ipa, &bundle_id, progress).await
}

/// AFC upload + installation_proxy upgrade over any lockdown-reachable provider
/// (usbmux USB/network, or a Wi-Fi TcpProvider).
async fn install_via_lockdown<F, Fut>(
    provider: &dyn IdeviceProvider,
    signed_ipa: &Path,
    bundle_id: &str,
    progress: &F,
) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    let mut afc = AfcClient::connect(provider)
        .await
        .map_err(|e| anyhow::anyhow!("AFC: {e:?}"))?;
    upload_ipa(&mut afc, signed_ipa, progress).await?;
    let mut inst = InstallationProxyClient::connect(provider)
        .await
        .map_err(|e| anyhow::anyhow!("InstallationProxy: {e:?}"))?;
    upgrade(&mut inst, bundle_id, progress).await
}

/// Uploads the IPA into the AFC media jail at `PublicStaging/homesign.ipa`.
/// Works for both USB-direct and tunneled AFC clients. Upload = 0–85% of the phase.
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
        // write_entire actively pumps the transport (unlike AsyncWrite, which stalls
        // on the userspace tunnel).
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

/// Wireless install over the iOS 17+ RemoteXPC tunnel.
async fn install_over_tunnel<F, Fut>(
    device: &Device,
    signed_ipa: &Path,
    bundle_id: &str,
    progress: F,
) -> anyhow::Result<()>
where
    F: Fn(u64, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    let rp_path = crate::pairing::rp_pairing_path(&device.pairing_path);
    if !rp_path.exists() {
        anyhow::bail!(
            "bezdrátová instalace vyžaduje RemotePairing — připoj iPad kabelem a spáruj znovu \
             (tím se vytvoří) nebo instaluj přes USB"
        );
    }

    progress(2, "Hledám iPad na síti (Bonjour)…".into()).await;
    let (host, rsd_service_port) = discover_remoted()
        .await
        .ok_or_else(|| anyhow::anyhow!("iPad nenalezen přes Bonjour (_remoted._tcp)"))?;
    tracing::info!("_remoted._tcp: {host}:{rsd_service_port}");

    // 1) Reach the untrusted tunnel service over the device's initial RSD.
    let stream = connect_host(&host, rsd_service_port)
        .await
        .ok_or_else(|| anyhow::anyhow!("nelze se připojit k {host}:{rsd_service_port}"))?;
    let handshake = RsdHandshake::new(stream)
        .await
        .map_err(|e| anyhow::anyhow!("RSD handshake: {e:?}"))?;
    let ts_port = handshake
        .services
        .get(TUNNEL_SERVICE)
        .map(|s| s.port)
        .ok_or_else(|| anyhow::anyhow!("tunnel service nenalezen"))?;
    let ts_stream = connect_host(&host, ts_port)
        .await
        .ok_or_else(|| anyhow::anyhow!("nelze se připojit k tunnel service"))?;
    let mut conn = RemoteXpcClient::new(ts_stream)
        .await
        .map_err(|e| anyhow::anyhow!("RemoteXPC init: {e:?}"))?;
    conn.do_handshake()
        .await
        .map_err(|e| anyhow::anyhow!("XPC handshake: {e:?}"))?;
    let _ = conn.recv_root().await;

    // 2) RemotePairing verify (using the record created over USB), yields the TLS-PSK key.
    progress(6, "Navazuji šifrovaný tunel…".into()).await;
    let mut rpf = RpPairingFile::read_from_file(&rp_path)
        .await
        .map_err(|e| anyhow::anyhow!("nelze načíst RP pairing file: {e:?}"))?;
    let mut rpc = RemotePairingClient::new(conn, RP_HOST);
    rpc.connect(&mut rpf, || async { "000000".to_string() })
        .await
        .map_err(|e| anyhow::anyhow!("RemotePairing verify: {e:?}"))?;

    // 3) Ask the device to open a tunnel port, then wrap it in TLS-PSK + CDTunnel.
    let tunnel_port = rpc
        .create_tcp_listener()
        .await
        .map_err(|e| anyhow::anyhow!("create_tcp_listener: {e:?}"))?;
    let tunnel_stream = connect_host(&host, tunnel_port)
        .await
        .ok_or_else(|| anyhow::anyhow!("nelze se připojit k tunel portu"))?;
    let tunnel = connect_tls_psk_tunnel_native(tunnel_stream, rpc.encryption_key())
        .await
        .map_err(|e| anyhow::anyhow!("TLS-PSK tunel: {e:?}"))?;

    let client_ip: IpAddr = tunnel.info.client_address.parse()?;
    let server_ip: IpAddr = tunnel.info.server_address.parse()?;
    let tunnel_rsd_port = tunnel.info.server_rsd_port;

    // 4) Userspace TCP stack over the tunnel, then RSD through it.
    let adapter = Adapter::new(Box::new(tunnel.into_inner()), client_ip, server_ip);
    let mut handle = adapter.to_async_handle();
    let rsd_stream = handle
        .connect(tunnel_rsd_port)
        .await
        .map_err(|e| anyhow::anyhow!("RSD přes tunel: {e:?}"))?;
    let mut tunneled = RsdHandshake::new(rsd_stream)
        .await
        .map_err(|e| anyhow::anyhow!("RSD handshake (tunel): {e:?}"))?;

    // 5) AFC upload + installation_proxy, both over the tunnel.
    let mut afc: AfcClient = tunneled
        .connect(&mut handle)
        .await
        .map_err(|e| anyhow::anyhow!("AFC (tunel): {e:?}"))?;
    upload_ipa(&mut afc, signed_ipa, &progress).await?;

    let mut inst: InstallationProxyClient = tunneled
        .connect(&mut handle)
        .await
        .map_err(|e| anyhow::anyhow!("InstallationProxy (tunel): {e:?}"))?;
    upgrade(&mut inst, bundle_id, &progress).await
}

/// Connects a TcpStream to `host:port`, resolving `.local` hostnames (which yields
/// scoped link-local IPv6 on macOS).
async fn connect_host(host: &str, port: u16) -> Option<TcpStream> {
    let mut last_err = None;
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => {
            // Prefer IPv6 (RemoteXPC is IPv6), then anything else.
            let mut sorted: Vec<_> = addrs.collect();
            sorted.sort_by_key(|a| a.is_ipv4());
            for a in sorted {
                match TcpStream::connect(a).await {
                    Ok(s) => return Some(s),
                    Err(e) => last_err = Some(e),
                }
            }
        }
        Err(e) => last_err = Some(e),
    }
    if let Some(e) = last_err {
        tracing::warn!("connect {host}:{port} selhalo: {e}");
    }
    None
}

/// Browses `_remoted._tcp` via the system `dns-sd` and returns the first
/// instance's (hostname, port). iOS 17+ advertises its RemoteXPC RSD here over
/// link-local IPv6.
///
/// NOTE: with several iOS devices on the LAN this picks the first one; matching a
/// specific UDID needs the Bonjour TXT auth-tag (idevice::mdns) and is a TODO.
async fn discover_remoted() -> Option<(String, u16)> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new("/usr/bin/dns-sd")
        .args(["-Z", "_remoted._tcp", "local"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let mut lines = BufReader::new(stdout).lines();

    let mut found: Option<(String, u16)> = None;
    let deadline = tokio::time::sleep(Duration::from_millis(3000));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        // SRV line, e.g. "Instance._remoted._tcp. SRV 0 0 49152 host.local."
                        if let Some(hit) = parse_srv(&l) {
                            found = Some(hit);
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
    }
    let _ = child.kill().await;
    found
}

/// Extracts (host, port) from a `dns-sd -Z` SRV line.
fn parse_srv(line: &str) -> Option<(String, u16)> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let srv = toks.iter().position(|t| *t == "SRV")?;
    // After SRV: priority weight port target
    let port = toks.get(srv + 3)?.parse::<u16>().ok()?;
    let target = toks.get(srv + 4)?.trim_end_matches('.').to_string();
    if target.is_empty() {
        return None;
    }
    Some((target, port))
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
/// USB only — a Wi-Fi health-check would need the full tunnel handshake.
pub async fn ping(device: &Device) -> anyhow::Result<()> {
    if let Some((provider, _)) = muxd_provider(device).await {
        let _proxy = CoreDeviceProxy::connect(provider.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("zařízení nedosažitelné: {e:?}"))?;
        return Ok(());
    }
    // Fall back to a plain TCP reach test on the known Wi-Fi address.
    let _p = tcp_provider(device)?;
    Ok(())
}
