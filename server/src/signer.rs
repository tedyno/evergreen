//! Podepisování IPA.
//!
//! Dva režimy:
//!  1. **Passthrough** — IPA už obsahuje `embedded.mobileprovision` (podepsané
//!     jinde). Nic nepřepisujeme, jen vytáhneme expiraci profilu pro scheduler.
//!     Tím je M1 (instalace po síti + sledování expirace) plně funkční.
//!  2. **Resign** — přepis bundle id na naše App ID, vložení profilu z portálu,
//!     podpis binárky i nested frameworků. To je M2 a vyžaduje přihlášený Apple
//!     účet + dokončený `devportal`; zatím vrací jasnou chybu, ať nepředstírá.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::models::Ipa;
use crate::state::AppState;

pub struct SignedIpa {
    pub path: PathBuf,
    pub signed_bundle_id: String,
    pub profile_expires: Option<DateTime<Utc>>,
}

pub async fn resign(st: &AppState, ipa: &Ipa) -> anyhow::Result<SignedIpa> {
    let src = PathBuf::from(&ipa.path);
    let data = tokio::fs::read(&src).await?;

    // Zjisti, jestli je IPA už podepsaná (má embedded profil).
    let bytes = data.clone();
    let embedded = tokio::task::spawn_blocking(move || extract_embedded_profile(&bytes)).await??;

    if let Some(profile) = embedded {
        // Passthrough — IPA je hotová, jen ji přeneseme dál a přečteme expiraci.
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

    // Resign vyžaduje přihlášený účet a hotový devportal (M2).
    if !st.apple.is_logged_in().await {
        anyhow::bail!(
            "IPA není podepsaná a Apple účet není přihlášený — buď nahraj už podepsané \
             IPA (passthrough), nebo se přihlas a počkej na dokončení M2 (resign)"
        );
    }
    anyhow::bail!(
        "resign (přepis bundle id + podpis) je M2 a čeká na dokončení devportal/codesign \
         proti živému účtu"
    )
}

/// Vytáhne `Payload/*.app/embedded.mobileprovision` z IPA, pokud existuje.
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

/// Provisioning profil je CMS/PKCS7 obálka kolem XML plistu. Vytáhneme plist a
/// přečteme `ExpirationDate`.
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

/// Najde `<?xml ... </plist>` uvnitř binární CMS obálky.
fn extract_plist_from_cms(data: &[u8]) -> Option<Vec<u8>> {
    let start = find_subslice(data, b"<?xml")?;
    let end_marker = b"</plist>";
    let end = find_subslice(&data[start..], end_marker)? + start + end_marker.len();
    Some(data[start..end].to_vec())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
