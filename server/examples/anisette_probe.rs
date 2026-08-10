//! Diagnostika: jaký anisette provider dostaneme na tomhle stroji a vrací
//! platné hlavičky? Na macOS by měl vyhrát nativní AOSKit (žádný sidecar).
//! Spuštění: `cargo run -p homesign-server --example anisette_probe`

use omnisette::{AnisetteConfiguration, AnisetteHeaders, AnisetteHeadersProviderType};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = AnisetteConfiguration::new();
    let mut res = AnisetteHeaders::get_anisette_headers_provider(cfg)
        .map_err(|e| anyhow::anyhow!("získání provideru selhalo: {e:?}"))?;

    let kind = match res.provider_type {
        AnisetteHeadersProviderType::Local => "Local (nativní, bez sidecaru)",
        AnisetteHeadersProviderType::Remote => "Remote (anisette server)",
    };
    println!("provider_type = {kind}");

    let headers = res
        .provider
        .get_authentication_headers()
        .await
        .map_err(|e| anyhow::anyhow!("get_authentication_headers: {e:?}"))?;

    println!("počet hlaviček: {}", headers.len());
    for key in ["X-Apple-I-MD", "X-Apple-I-MD-M", "X-Apple-I-MD-RINFO", "X-Mme-Client-Info"] {
        match headers.get(key) {
            Some(v) => println!("  {key}: {} znaků", v.len()),
            None => println!("  {key}: (chybí)"),
        }
    }
    Ok(())
}
