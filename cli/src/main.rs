//! homesign-cli — jednorázové párování iPadu (přes USB) a upload pairing filu na
//! server. Pak už zařízení instaluje/refreshuje server sám po Wi-Fi.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use idevice::lockdown::LockdownClient;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::IdeviceService;

#[derive(Parser)]
#[command(name = "homesign-cli", about = "Párování iPadu pro homesign")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Spáruj připojený iPad (USB) a nahraj pairing file na server.
    Pair {
        /// URL homesign serveru, např. http://192.168.1.10:8080
        #[arg(long)]
        server: String,
        /// IP adresa iPadu ve Wi-Fi síti (server přes ni instaluje). Doporučeno.
        #[arg(long)]
        address: Option<String>,
        /// Konkrétní UDID (jinak první USB zařízení).
        #[arg(long)]
        udid: Option<String>,
    },
    /// Jen vypiš připojená USB zařízení.
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::List => list().await,
        Cmd::Pair { server, address, udid } => pair(server, address, udid).await,
    }
}

async fn usbmuxd() -> Result<UsbmuxdConnection> {
    UsbmuxdConnection::default()
        .await
        .map_err(|e| anyhow!("nelze se připojit k usbmuxd (běží iTunes/Apple Mobile Device?): {e:?}"))
}

async fn list() -> Result<()> {
    let mut u = usbmuxd().await?;
    let devs = u.get_devices().await.map_err(|e| anyhow!("{e:?}"))?;
    let usb: Vec<_> = devs.into_iter().filter(|d| d.connection_type == Connection::Usb).collect();
    if usb.is_empty() {
        println!("Žádné zařízení přes USB.");
    }
    for d in usb {
        println!("{}", d.udid);
    }
    Ok(())
}

async fn pair(server: String, address: Option<String>, udid: Option<String>) -> Result<()> {
    let mut u = usbmuxd().await?;
    let dev = match &udid {
        Some(udid) => u.get_device(udid).await.map_err(|e| anyhow!("zařízení {udid}: {e:?}"))?,
        None => u
            .get_devices()
            .await
            .map_err(|e| anyhow!("{e:?}"))?
            .into_iter()
            .find(|d| d.connection_type == Connection::Usb)
            .ok_or_else(|| anyhow!("žádné USB zařízení — připoj iPad kabelem a odemkni"))?,
    };
    println!("Zařízení: {}", dev.udid);

    let provider = dev.to_provider(UsbmuxdAddr::default(), "homesign-pair");
    let mut lockdown = LockdownClient::connect(&provider)
        .await
        .map_err(|e| anyhow!("lockdown: {e:?}"))?;

    // Spáruj.
    let host_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let buid = u.get_buid().await.map_err(|e| anyhow!("buid: {e:?}"))?;
    let mut pairing_file = lockdown
        .pair(host_id, buid, Some("homesign"))
        .await
        .map_err(|e| anyhow!("párování selhalo (potvrď 'Trust' na iPadu): {e:?}"))?;

    // Otestuj pairing file.
    lockdown
        .start_session(&pairing_file)
        .await
        .map_err(|e| anyhow!("test pairing filu selhal: {e:?}"))?;

    // Zapni Wi-Fi connections, ať server dosáhne na iPad bez kabelu.
    if let Err(e) = lockdown
        .set_value(
            "EnableWifiConnections",
            true.into(),
            Some("com.apple.mobile.wireless_lockdown"),
        )
        .await
    {
        eprintln!("varování: nepodařilo se zapnout Wi-Fi connections: {e:?}");
    }

    // Metadata pro server.
    let name = get_str(&mut lockdown, "DeviceName").await.unwrap_or_else(|| "iPad".into());
    let version = get_str(&mut lockdown, "ProductVersion").await;

    pairing_file.udid = Some(dev.udid.clone());
    let serialized = pairing_file.serialize().map_err(|e| anyhow!("serialize: {e:?}"))?;

    // Ulož i lokálně do usbmuxd (jitterbug spec).
    let _ = u.save_pair_record(&dev.udid, serialized.clone()).await;

    // Upload na server.
    let url = format!("{}/api/pair", server.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = client
        .post(&url)
        .header("X-UDID", &dev.udid)
        .header("X-Name", &name)
        .header("Content-Type", "application/x-plist")
        .body(serialized);
    if let Some(addr) = &address {
        req = req.header("X-Address", addr);
    }
    if let Some(v) = &version {
        req = req.header("X-IOS-Version", v);
    }
    let resp = req.send().await.context("upload na server selhal")?;
    if !resp.status().is_success() {
        return Err(anyhow!("server vrátil {}: {}", resp.status(), resp.text().await.unwrap_or_default()));
    }

    println!("✓ Spárováno a nahráno na server.");
    println!("  Zařízení: {name} (iOS {})", version.as_deref().unwrap_or("?"));
    if address.is_none() {
        println!("  ⚠ Bez --address server nezná IP iPadu. Přidej ji ve web UI nebo spusť znovu s --address.");
    }
    Ok(())
}

async fn get_str(lockdown: &mut LockdownClient, key: &str) -> Option<String> {
    match lockdown.get_value(Some(key), None).await {
        Ok(plist::Value::String(s)) => Some(s),
        _ => None,
    }
}
