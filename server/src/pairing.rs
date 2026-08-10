//! Pairing an iPad over USB — directly in the server (which runs on macOS inside the
//! Mac app), so no CLI is needed. The app only calls a REST endpoint.
//!
//! One extra step over bare pairing: after enabling Wi-Fi connections we look up the
//! iPad's IP in the ARP table by its Wi-Fi MAC, so the user doesn't have to enter it manually.

use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use idevice::lockdown::LockdownClient;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::IdeviceService;

/// Pairing result for the API.
#[derive(Debug, serde::Serialize)]
pub struct PairResult {
    pub udid: String,
    pub name: String,
    pub ios_version: Option<String>,
    pub address: Option<String>,
    pub wifi_mac: Option<String>,
}

async fn usbmuxd() -> Result<UsbmuxdConnection> {
    UsbmuxdConnection::default()
        .await
        .map_err(|e| anyhow!("nelze se připojit k usbmuxd (běží Apple Mobile Device?): {e:?}"))
}

/// UDID of the first connected USB device (or all of them).
pub async fn list_usb() -> Result<Vec<String>> {
    let mut u = usbmuxd().await?;
    let devs = u.get_devices().await.map_err(|e| anyhow!("{e:?}"))?;
    Ok(devs
        .into_iter()
        .filter(|d| d.connection_type == Connection::Usb)
        .map(|d| d.udid)
        .collect())
}

/// Pairs the connected iPad, enables Wi-Fi connections, discovers the IP and returns
/// metadata + the serialized pairing file (for the caller to store).
pub async fn pair_usb(udid: Option<&str>) -> Result<(PairResult, Vec<u8>)> {
    let mut u = usbmuxd().await?;
    let dev = match udid {
        Some(udid) => u.get_device(udid).await.map_err(|e| anyhow!("zařízení {udid}: {e:?}"))?,
        None => u
            .get_devices()
            .await
            .map_err(|e| anyhow!("{e:?}"))?
            .into_iter()
            .find(|d| d.connection_type == Connection::Usb)
            .ok_or_else(|| anyhow!("žádné USB zařízení — připoj iPad kabelem a odemkni"))?,
    };

    let provider = dev.to_provider(UsbmuxdAddr::default(), "homesign-pair");
    let mut lockdown = LockdownClient::connect(&provider)
        .await
        .map_err(|e| anyhow!("lockdown: {e:?}"))?;

    // Pair.
    let host_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let buid = u.get_buid().await.map_err(|e| anyhow!("buid: {e:?}"))?;
    let mut pairing_file = lockdown
        .pair(host_id, buid, Some("homesign"))
        .await
        .map_err(|e| anyhow!("párování selhalo (potvrď 'Trust' na iPadu): {e:?}"))?;

    lockdown
        .start_session(&pairing_file)
        .await
        .map_err(|e| anyhow!("test pairing filu selhal: {e:?}"))?;

    // Enable Wi-Fi connections so the server can reach the iPad without a cable.
    if let Err(e) = lockdown
        .set_value(
            "EnableWifiConnections",
            true.into(),
            Some("com.apple.mobile.wireless_lockdown"),
        )
        .await
    {
        tracing::warn!("nepodařilo se zapnout Wi-Fi connections: {e:?}");
    }

    let name = get_str(&mut lockdown, "DeviceName").await.unwrap_or_else(|| "iPad".into());
    let version = get_str(&mut lockdown, "ProductVersion").await;
    let wifi_mac = get_str(&mut lockdown, "WiFiAddress").await;

    pairing_file.udid = Some(dev.udid.clone());
    let serialized = pairing_file.serialize().map_err(|e| anyhow!("serialize: {e:?}"))?;
    let _ = u.save_pair_record(&dev.udid, serialized.clone()).await;

    // IP auto-detection: primarily from usbmuxd (network device), ARP fallback.
    let address = discover_ip(&dev.udid, wifi_mac.as_deref()).await;

    Ok((
        PairResult {
            udid: dev.udid.clone(),
            name,
            ios_version: version,
            address,
            wifi_mac,
        },
        serialized,
    ))
}

async fn get_str(lockdown: &mut LockdownClient, key: &str) -> Option<String> {
    match lockdown.get_value(Some(key), None).await {
        Ok(plist::Value::String(s)) => Some(s),
        _ => None,
    }
}

/// Discovers the iPad's IPv4 after enabling Wi-Fi connections. Sources in order of reliability:
/// 1) usbmuxd network device (`Connection::Network`) — has the IP directly;
/// 2) ARP by Wi-Fi MAC — the MAC is either known from pairing, or we extract it from Bonjour
///    (`_apple-mobdev2._tcp` advertises the iOS device's network MAC).
async fn discover_ip(udid: &str, mac: Option<&str>) -> Option<String> {
    // MAC for ARP: either from pairing, or from Bonjour (the iOS device's network MAC).
    let mut macs: Vec<[u8; 6]> = mac.and_then(parse_mac).into_iter().collect();
    if macs.is_empty() {
        macs = bonjour_ios_macs().await;
    }

    for attempt in 0..10 {
        if let Some(ip) = usbmux_network_ip(udid).await {
            return Some(ip);
        }
        for m in &macs {
            if let Some(ip) = arp_lookup(m).await {
                return Some(ip);
            }
        }
        // After a few fruitless attempts, nudge the network with a multicast ping (populates ARP).
        if attempt == 2 {
            let _ = tokio::process::Command::new("/sbin/ping")
                .args(["-c", "2", "-t", "1", "224.0.0.1"])
                .output()
                .await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}

/// Wi-Fi MACs of iOS devices advertised over Bonjour (`_apple-mobdev2._tcp`).
/// Uses the system `dns-sd` (reliable, unlike the mDNS libraries in Rust).
async fn bonjour_ios_macs() -> Vec<[u8; 6]> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = match tokio::process::Command::new("/usr/bin/dns-sd")
        .args(["-B", "_apple-mobdev2._tcp", "local"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let Some(stdout) = child.stdout.take() else { return vec![] };
    let mut lines = BufReader::new(stdout).lines();

    let mut macs: Vec<[u8; 6]> = Vec::new();
    let deadline = tokio::time::sleep(Duration::from_millis(2500));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        // The instance name (last token) is "MAC@...".
                        if let Some(inst) = l.split_whitespace().last() {
                            if let Some(mac_part) = inst.split('@').next() {
                                if let Some(m) = parse_mac(mac_part) {
                                    if !macs.contains(&m) {
                                        macs.push(m);
                                    }
                                }
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
    }
    let _ = child.kill().await;
    macs
}

/// Re-discovers the IP of an already-paired device (without re-pairing) — for the
/// "Zjistit IP" button on a device with an unknown address.
pub async fn detect_ip(udid: &str) -> Option<String> {
    discover_ip(udid, None).await
}

/// The iPad's IP from the usbmuxd device list (network connection for the given UDID).
async fn usbmux_network_ip(udid: &str) -> Option<String> {
    let mut u = usbmuxd().await.ok()?;
    let devs = u.get_devices().await.ok()?;
    devs.into_iter().find_map(|d| match d.connection_type {
        Connection::Network(ip) if d.udid == udid => Some(ip.to_string()),
        _ => None,
    })
}

/// MAC "aa:72:8d:a7:3e:1c" → bytes. Also handles the form without leading zeros (ARP output).
fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        out[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(out)
}

/// Finds the IPv4 for the given MAC in `arp -an` (byte comparison because of the zero formatting).
async fn arp_lookup(target: &[u8; 6]) -> Option<String> {
    let out = tokio::process::Command::new("/usr/sbin/arp").arg("-an").output().await.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // Format: "? (10.0.1.105) at aa:72:8d:a7:3e:1c on en0 ifscope [ethernet]"
        let (Some(l), Some(r)) = (line.find('('), line.find(')')) else { continue };
        let ip = &line[l + 1..r];
        if IpAddr::from_str(ip).map(|a| a.is_ipv4()).unwrap_or(false) {
            if let Some(at) = line.find(" at ") {
                let mac_str = line[at + 4..].split_whitespace().next().unwrap_or("");
                if parse_mac(mac_str).as_ref() == Some(target) {
                    return Some(ip.to_string());
                }
            }
        }
    }
    None
}
