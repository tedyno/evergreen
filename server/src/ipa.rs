//! Working with IPAs: storing the uploaded file + extracting metadata from Info.plist.

use std::io::Read;

use crate::error::{AppError, AppResult};
use crate::models::Ipa;
use crate::state::AppState;

/// Metadata extracted from `Payload/*.app/Info.plist`.
#[derive(Debug, Default)]
pub struct IpaMeta {
    pub bundle_id: String,
    pub name: String,
    pub version: Option<String>,
    pub icon: Option<Vec<u8>>,
}

/// Stores the uploaded IPA on disk, parses its metadata and writes it into the DB.
pub async fn store_uploaded(st: &AppState, filename: &str, data: &[u8]) -> AppResult<Ipa> {
    let id = uuid::Uuid::new_v4().to_string();
    let safe_name = sanitize(filename);
    let path = st.cfg.ipa_dir().join(format!("{id}-{safe_name}"));
    tokio::fs::write(&path, data)
        .await
        .map_err(|e| AppError::Other(e.into()))?;

    let bytes = data.to_vec();
    let meta = tokio::task::spawn_blocking(move || parse_meta(&bytes))
        .await
        .map_err(|e| AppError::Other(e.into()))?
        .map_err(AppError::BadRequest)?;

    let mut icon_path: Option<String> = None;
    if let Some(icon) = &meta.icon {
        let p = st.cfg.ipa_dir().join(format!("{id}.png"));
        if tokio::fs::write(&p, icon).await.is_ok() {
            icon_path = Some(p.to_string_lossy().to_string());
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let ipa = Ipa {
        id: id.clone(),
        filename: safe_name,
        bundle_id: meta.bundle_id,
        name: meta.name,
        version: meta.version,
        size_bytes: data.len() as i64,
        path: path.to_string_lossy().to_string(),
        icon_path,
        created_at: now.clone(),
    };

    sqlx::query(
        "INSERT INTO ipa (id, filename, bundle_id, name, version, size_bytes, path, icon_path, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&ipa.id)
    .bind(&ipa.filename)
    .bind(&ipa.bundle_id)
    .bind(&ipa.name)
    .bind(&ipa.version)
    .bind(ipa.size_bytes)
    .bind(&ipa.path)
    .bind(&ipa.icon_path)
    .bind(&ipa.created_at)
    .execute(&st.db)
    .await?;

    Ok(ipa)
}

/// Parses the main app's Info.plist inside the IPA (zip).
pub fn parse_meta(data: &[u8]) -> Result<IpaMeta, String> {
    let reader = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("není platný zip: {e}"))?;

    // Find Payload/<App>.app/Info.plist (the shortest path = the root bundle).
    let mut info_name: Option<String> = None;
    for i in 0..zip.len() {
        let name = zip.by_index(i).map_err(|e| e.to_string())?.name().to_string();
        if is_root_info_plist(&name) {
            match &info_name {
                Some(cur) if cur.len() <= name.len() => {}
                _ => info_name = Some(name),
            }
        }
    }
    let info_name = info_name.ok_or("Info.plist nenalezen v Payload/*.app")?;

    let mut buf = Vec::new();
    zip.by_name(&info_name)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;

    let val: plist::Value = plist::from_bytes(&buf).map_err(|e| e.to_string())?;
    let dict = val.as_dictionary().ok_or("Info.plist není dictionary")?;

    let bundle_id = dict
        .get("CFBundleIdentifier")
        .and_then(|v| v.as_string())
        .ok_or("chybí CFBundleIdentifier")?
        .to_string();
    let name = dict
        .get("CFBundleDisplayName")
        .or_else(|| dict.get("CFBundleName"))
        .and_then(|v| v.as_string())
        .unwrap_or(&bundle_id)
        .to_string();
    let version = dict
        .get("CFBundleShortVersionString")
        .and_then(|v| v.as_string())
        .map(|s| s.to_string());

    // Icon: first try the first CFBundleIcons file, otherwise skip.
    let app_dir = info_name.trim_end_matches("Info.plist");
    let icon = extract_icon(&mut zip, app_dir, dict);

    Ok(IpaMeta { bundle_id, name, version, icon })
}

fn is_root_info_plist(name: &str) -> bool {
    // Payload/Foo.app/Info.plist — not in a .appex or a nested framework.
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() == 3
        && parts[0] == "Payload"
        && parts[1].ends_with(".app")
        && parts[2] == "Info.plist"
}

fn extract_icon(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    app_dir: &str,
    dict: &plist::Dictionary,
) -> Option<Vec<u8>> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(icons) = dict.get("CFBundleIcons").and_then(|v| v.as_dictionary()) {
        if let Some(primary) = icons
            .get("CFBundlePrimaryIcon")
            .and_then(|v| v.as_dictionary())
        {
            if let Some(files) = primary.get("CFBundleIconFiles").and_then(|v| v.as_array()) {
                for f in files {
                    if let Some(s) = f.as_string() {
                        candidates.push(s.to_string());
                    }
                }
            }
        }
    }
    // Try the icon name variants (@2x, @3x).
    for base in candidates.iter().rev() {
        for suffix in ["@3x.png", "@2x.png", ".png", "@2x~ipad.png"] {
            let path = format!("{app_dir}{base}{suffix}");
            if let Ok(mut f) = zip.by_name(&path) {
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                    return Some(buf);
                }
            }
        }
    }
    None
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}
