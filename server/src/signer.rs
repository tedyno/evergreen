//! Signing IPAs.
//!
//! Two modes:
//!  1. **Passthrough** — the IPA already contains `embedded.mobileprovision` (signed
//!     elsewhere). We rewrite nothing, we only read the profile's expiration for the scheduler.
//!     This makes M1 (installation over the network + expiration tracking) fully functional.
//!  2. **Resign** — rewrite the bundle id to our App ID, insert the profile from the portal,
//!     sign the binary and nested frameworks. This is M2 and requires a logged-in Apple
//!     account + a finished `devportal`; for now it returns a clear error so it doesn't pretend.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::models::{Device, Ipa};
use crate::state::AppState;

pub struct SignedIpa {
    pub path: PathBuf,
    pub signed_bundle_id: String,
    pub profile_expires: Option<DateTime<Utc>>,
}

pub async fn resign(st: &AppState, ipa: &Ipa, device: &Device) -> anyhow::Result<SignedIpa> {
    let src = PathBuf::from(&ipa.path);
    let data = tokio::fs::read(&src).await?;

    // Determine whether the IPA is already signed (has an embedded profile).
    let bytes = data.clone();
    let embedded = tokio::task::spawn_blocking(move || extract_embedded_profile(&bytes)).await??;

    if let Some(profile) = embedded {
        // Passthrough — the IPA is ready, we just pass it along and read the expiration.
        let expires = parse_profile_expiration(&profile);
        tracing::info!(
            "IPA {} je už podepsaná, passthrough (expirace: {:?})",
            ipa.name, expires
        );
        return Ok(SignedIpa {
            path: src,
            signed_bundle_id: ipa.bundle_id.clone(),
            profile_expires: expires,
        });
    }

    // Do we already have a signed build with a valid profile? → install without auth/provisioning.
    let cached = st.cfg.ipa_dir().join(format!("{}-signed.ipa", ipa.id));
    if cached.exists() {
        let cbytes = tokio::fs::read(&cached).await?;
        let cb = cbytes.clone();
        if let Ok(Some(profile)) =
            tokio::task::spawn_blocking(move || extract_embedded_profile(&cb)).await?
        {
            let expires = parse_profile_expiration(&profile);
            let still_valid = expires.map(|e| e > Utc::now()).unwrap_or(false);
            if still_valid {
                let meta = tokio::task::spawn_blocking(move || crate::ipa::parse_meta(&cbytes)).await?;
                let bid = meta.map(|m| m.bundle_id).unwrap_or_else(|_| ipa.bundle_id.clone());
                tracing::info!("reuse podepsaného buildu {} (bez auth)", ipa.name);
                return Ok(SignedIpa {
                    path: cached,
                    signed_bundle_id: bid,
                    profile_expires: expires,
                });
            }
        }
    }

    // Resign requires a logged-in account.
    if !st.apple.is_logged_in().await {
        anyhow::bail!(
            "IPA není podepsaná a Apple účet není přihlášený — buď nahraj už podepsané \
             IPA (passthrough), nebo se přihlas"
        );
    }

    crate::codesign::provision_and_sign(st, ipa, device).await
}

/// Extracts `Payload/*.app/embedded.mobileprovision` from the IPA, if present.
fn extract_embedded_profile(data: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    let reader = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(reader)?;
    let mut target: Option<String> = None;
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_string();
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() == 3
            && parts[0] == "Payload"
            && parts[1].ends_with(".app")
            && parts[2] == "embedded.mobileprovision"
        {
            target = Some(name);
            break;
        }
    }
    let Some(name) = target else { return Ok(None) };
    let mut buf = Vec::new();
    zip.by_name(&name)?.read_to_end(&mut buf)?;
    Ok(Some(buf))
}

/// Public wrapper for the codesign module.
pub fn parse_profile_expiration_pub(profile: &[u8]) -> Option<DateTime<Utc>> {
    parse_profile_expiration(profile)
}

/// A provisioning profile is a CMS/PKCS7 envelope around an XML plist. We extract
/// the plist and read `ExpirationDate`.
fn parse_profile_expiration(profile: &[u8]) -> Option<DateTime<Utc>> {
    let plist_bytes = extract_plist_from_cms(profile)?;
    let val: plist::Value = plist::from_bytes(&plist_bytes).ok()?;
    let dict = val.as_dictionary()?;
    let exp = dict.get("ExpirationDate")?;
    // plist::Value::Date → SystemTime
    if let plist::Value::Date(d) = exp {
        let st: std::time::SystemTime = (*d).into();
        return Some(DateTime::<Utc>::from(st));
    }
    None
}

/// Finds `<?xml ... </plist>` inside the binary CMS envelope.
fn extract_plist_from_cms(data: &[u8]) -> Option<Vec<u8>> {
    let start = find_subslice(data, b"<?xml")?;
    let end_marker = b"</plist>";
    let end = find_subslice(&data[start..], end_marker)? + start + end_marker.len();
    Some(data[start..end].to_vec())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
