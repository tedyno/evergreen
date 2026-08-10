//! Resign pipeline pro macOS: provisioning přes Developer Services (devportal)
//! + vlastní podpis systémovými nástroji (`openssl`, `security`, `codesign`).
//!
//! Homesign varianta B běží jen na macOS, takže je nejrobustnější použít Apple
//! vlastní `codesign` (zvládne nested frameworky/extensions i CodeResources) než
//! reimplementovat podpis v Rustu.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::apple::{devportal, XcodeAuth};
use crate::models::{Device, Ipa};
use crate::signer::SignedIpa;
use crate::state::AppState;

// Absolutní cesty — GUI appka má osekaný PATH.
const OPENSSL: &str = "/usr/bin/openssl";
const SECURITY: &str = "/usr/bin/security";
const CODESIGN: &str = "/usr/bin/codesign";
const UNZIP: &str = "/usr/bin/unzip";
const ZIP: &str = "/usr/bin/zip";

/// Celý resign tok: provisioning + podpis. Vrací podepsanou IPA.
pub async fn provision_and_sign(st: &AppState, ipa: &Ipa, device: &Device) -> Result<SignedIpa> {
    // Offline cesta: máme-li na disku cert+klíč+profil (a profil je platný),
    // přepodepíšeme bez jediného volání Apple (obchází i throttle -22411).
    if let Some(signed) = try_offline_resign(st, ipa).await? {
        return Ok(signed);
    }

    let auth = st.apple.xcode_auth().await.context("získání Xcode auth")?;
    let team_id = devportal::first_team_id(&auth).await.context("listTeams")?;

    // 1) Zaregistruj zařízení (idempotentně).
    devportal::add_device(&auth, &team_id, &device.udid, &device.name)
        .await
        .context("registrace zařízení")?;

    // 2) Zajisti podpisovou identitu (cert + klíč), reuse z disku.
    let identity = ensure_identity(st, &auth, &team_id).await.context("certifikát")?;

    // Původní bundle id patří vývojáři appky (např. com.shapr3d.shapr) a nejde
    // zaregistrovat na náš účet. Přepíšeme ho na unikátní pod naším týmem —
    // deterministicky (stejné při refreshi → upgrade, ne duplikát).
    let new_bundle_id = format!("{}.{}", ipa.bundle_id, team_id.to_lowercase());

    // 3) Zajisti App ID pro nový bundle id.
    let app_id_id = devportal::ensure_app_id(&auth, &team_id, &new_bundle_id, &ipa.name)
        .await
        .context("App ID")?;

    // 4) Stáhni provisioning profil (a ulož pro budoucí offline resign/refresh).
    let profile = devportal::download_profile(&auth, &team_id, &app_id_id)
        .await
        .context("provisioning profil")?;
    let _ = tokio::fs::write(signing_dir(st).join("profile.mobileprovision"), &profile).await;

    // 5) Podepiš IPA (přepíše bundle id v Info.plist).
    let signed_path = sign_ipa(st, ipa, &identity, &profile, &new_bundle_id).await?;

    let profile_expires = crate::signer::parse_profile_expiration_pub(&profile);
    Ok(SignedIpa {
        path: signed_path,
        signed_bundle_id: new_bundle_id,
        profile_expires,
    })
}

/// Offline resign z uložených cert+klíč+profil — bez volání Apple. None, když
/// něco chybí nebo profil vypršel.
async fn try_offline_resign(st: &AppState, ipa: &Ipa) -> Result<Option<SignedIpa>> {
    let dir = signing_dir(st);
    let key_pem = dir.join("key.pem");
    let cert_der = dir.join("cert.der");
    let profile_path = dir.join("profile.mobileprovision");
    if !key_pem.exists() || !cert_der.exists() || !profile_path.exists() {
        return Ok(None);
    }

    let profile = tokio::fs::read(&profile_path).await?;
    // Platnost profilu.
    let expires = crate::signer::parse_profile_expiration_pub(&profile);
    if expires.map(|e| e <= chrono::Utc::now()).unwrap_or(true) {
        tracing::info!("offline profil vypršel/neznámý — použiju online cestu");
        return Ok(None);
    }

    // new_bundle_id z application-identifier ("{TEAMID}.{bundleid}").
    let Some(new_bundle_id) = profile_bundle_id(&profile) else {
        return Ok(None);
    };

    let sha1 = cert_sha1(&cert_der).await?;
    let identity = Identity { cert_der_path: cert_der, key_pem_path: key_pem, sha1 };

    tracing::info!("OFFLINE resign (bez auth) → {new_bundle_id}");
    let signed_path = sign_ipa(st, ipa, &identity, &profile, &new_bundle_id).await?;
    Ok(Some(SignedIpa {
        path: signed_path,
        signed_bundle_id: new_bundle_id,
        profile_expires: expires,
    }))
}

/// Vytáhne bundle id z profilu (Entitlements → application-identifier bez team prefixu).
fn profile_bundle_id(profile: &[u8]) -> Option<String> {
    let start = find(profile, b"<?xml")?;
    let end = find(&profile[start..], b"</plist>")? + start + b"</plist>".len();
    let val: plist::Value = plist::from_bytes(&profile[start..end]).ok()?;
    let app_id = val
        .as_dictionary()?
        .get("Entitlements")?
        .as_dictionary()?
        .get("application-identifier")?
        .as_string()?;
    // "{TEAMID}.{bundleid}" → bundleid
    app_id.split_once('.').map(|(_, b)| b.to_string())
}

/// Podpisová identita uložená na disku (cert DER + privátní klíč PEM).
struct Identity {
    cert_der_path: PathBuf,
    key_pem_path: PathBuf,
    sha1: String,
}

fn signing_dir(st: &AppState) -> PathBuf {
    st.cfg.data_dir.join("signing")
}

/// Reuse cert+klíč z disku, jinak vygeneruj CSR a získej cert z účtu.
async fn ensure_identity(st: &AppState, auth: &XcodeAuth, team_id: &str) -> Result<Identity> {
    let dir = signing_dir(st);
    tokio::fs::create_dir_all(&dir).await?;
    let key_pem = dir.join("key.pem");
    let cert_der = dir.join("cert.der");

    // 1) Máme-li klíč, zkus k němu dohledat vydaný cert na účtu (bez plýtvání
    //    dalším certem z free limitu).
    if key_pem.exists() {
        if let Ok(our_mod) = key_modulus(&key_pem).await {
            if let Ok(certs) = devportal::list_dev_certs(auth, team_id).await {
                for c in certs {
                    if cert_der_modulus(&c.cert_der).await.ok().as_deref() == Some(our_mod.as_str()) {
                        tokio::fs::write(&cert_der, &c.cert_der).await?;
                        let sha1 = cert_sha1(&cert_der).await?;
                        return Ok(Identity { cert_der_path: cert_der, key_pem_path: key_pem, sha1 });
                    }
                }
            }
        }
    }

    // 2) Jinak vygeneruj nový klíč + CSR a získej cert.
    let csr_pem = dir.join("csr.pem");
    run(
        OPENSSL,
        &[
            "req", "-new", "-newkey", "rsa:2048", "-nodes",
            "-keyout", key_pem.to_str().unwrap(),
            "-out", csr_pem.to_str().unwrap(),
            "-subj", "/CN=homesign/O=homesign/C=US",
        ],
    )
    .await
    .context("openssl CSR")?;

    let csr = tokio::fs::read_to_string(&csr_pem).await?;
    let machine_id = machine_id(st).await?;
    devportal::submit_csr(auth, team_id, &csr, &machine_id, "homesign")
        .await
        .context("submitCSR")?;

    // Vyber z účtu cert, jehož veřejný klíč sedí s naším privátním (spolehlivé).
    let our_mod = key_modulus(&key_pem).await?;
    let certs = devportal::list_dev_certs(auth, team_id).await.context("listCerts")?;
    let mut matched: Option<Vec<u8>> = None;
    for c in certs {
        if cert_der_modulus(&c.cert_der).await.ok().as_deref() == Some(our_mod.as_str()) {
            matched = Some(c.cert_der);
            break;
        }
    }
    let der = matched.ok_or_else(|| anyhow!("vydaný cert k našemu klíči nenalezen (limit certů?)"))?;
    tokio::fs::write(&cert_der, &der).await?;

    let sha1 = cert_sha1(&cert_der).await?;
    Ok(Identity { cert_der_path: cert_der, key_pem_path: key_pem, sha1 })
}

/// Modulus RSA privátního klíče (hex) — pro párování s certem.
async fn key_modulus(key_pem: &Path) -> Result<String> {
    let out = output(OPENSSL, &["rsa", "-in", key_pem.to_str().unwrap(), "-noout", "-modulus"]).await?;
    Ok(out.trim().to_string())
}

/// Modulus veřejného klíče v certifikátu (DER).
async fn cert_der_modulus(cert_der: &[u8]) -> Result<String> {
    // openssl čte DER ze stdin.
    let tmp = std::env::temp_dir().join(format!("hs-cert-{}.der", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, cert_der).await?;
    let out = output(
        OPENSSL,
        &["x509", "-inform", "DER", "-in", tmp.to_str().unwrap(), "-noout", "-modulus"],
    )
    .await;
    let _ = tokio::fs::remove_file(&tmp).await;
    Ok(out?.trim().to_string())
}

/// Sedí privátní klíč s certifikátem?
async fn key_matches_cert(key_pem: &Path, cert_der: &Path) -> Result<bool> {
    let km = key_modulus(key_pem).await?;
    let der = tokio::fs::read(cert_der).await?;
    let cm = cert_der_modulus(&der).await?;
    Ok(km == cm)
}

/// SHA-1 fingerprint certifikátu (identita pro `codesign -s`).
async fn cert_sha1(cert_der: &Path) -> Result<String> {
    let out = output(
        OPENSSL,
        &["x509", "-inform", "DER", "-in", cert_der.to_str().unwrap(), "-noout", "-fingerprint", "-sha1"],
    )
    .await?;
    // "SHA1 Fingerprint=AA:BB:..." → "AABB..."
    let hex = out
        .split('=')
        .nth(1)
        .ok_or_else(|| anyhow!("fingerprint parse"))?
        .trim()
        .replace(':', "");
    Ok(hex)
}

async fn machine_id(st: &AppState) -> Result<String> {
    let path = signing_dir(st).join("machine_id");
    if let Ok(s) = tokio::fs::read_to_string(&path).await {
        if !s.trim().is_empty() {
            return Ok(s.trim().to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let _ = tokio::fs::write(&path, &id).await;
    Ok(id)
}

/// Rozbalí IPA, vloží profil + entitlements a podepíše `codesign`em; vrátí .ipa.
async fn sign_ipa(
    st: &AppState,
    ipa: &Ipa,
    identity: &Identity,
    profile: &[u8],
    new_bundle_id: &str,
) -> Result<PathBuf> {
    let work = st.cfg.data_dir.join("signwork").join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&work).await?;

    // Rozbal IPA.
    run(UNZIP, &["-q", &ipa.path, "-d", work.to_str().unwrap()]).await.context("unzip IPA")?;

    // Najdi Payload/*.app.
    let payload = work.join("Payload");
    let app_dir = first_app_bundle(&payload).await?;

    // Přepiš CFBundleIdentifier v Info.plist na nový bundle id.
    rewrite_bundle_id(&app_dir.join("Info.plist"), new_bundle_id).await
        .context("přepis bundle id v Info.plist")?;

    // Vlož provisioning profil.
    tokio::fs::write(app_dir.join("embedded.mobileprovision"), profile).await?;

    // Entitlements z profilu.
    let entitlements = extract_entitlements(profile)
        .ok_or_else(|| anyhow!("profil neobsahuje Entitlements"))?;
    let ent_path = work.join("entitlements.plist");
    tokio::fs::write(&ent_path, entitlements).await?;

    // Připrav dočasný keychain s identitou.
    let keychain = Keychain::create(st, identity).await?;

    // Podepiš vnořené frameworky/dylib/extensions jednotlivě (od nejhlubšího),
    // pak teprve hlavní app s entitlements. `--deep` je nespolehlivý (nechává
    // některé frameworky nepodepsané → ApplicationVerificationFailed).
    let sign_res = sign_all(&app_dir, &ent_path, &identity.sha1, &keychain.path).await;

    keychain.cleanup().await;
    sign_res.context("codesign")?;

    // Zabal zpět do .ipa.
    let out_ipa = st.cfg.ipa_dir().join(format!("{}-signed.ipa", ipa.id));
    let _ = tokio::fs::remove_file(&out_ipa).await;
    // zip musí běžet z work adresáře, ať je cesta "Payload/...".
    run_in(&work, ZIP, &["-qr", out_ipa.to_str().unwrap(), "Payload"]).await.context("zip IPA")?;

    let _ = tokio::fs::remove_dir_all(&work).await;
    Ok(out_ipa)
}

/// Podepíše všechny vnořené frameworky/dylib/appex (od nejhlubšího), pak app.
async fn sign_all(app_dir: &Path, ent_path: &Path, sha1: &str, keychain: &Path) -> Result<()> {
    // Najdi vnořený podepsatelný kód.
    let out = output(
        "/usr/bin/find",
        &[
            app_dir.to_str().unwrap(),
            "(", "-name", "*.framework", "-o", "-name", "*.dylib", "-o", "-name", "*.appex", ")",
        ],
    )
    .await
    .unwrap_or_default();

    let mut items: Vec<String> = out.lines().map(|s| s.to_string()).filter(|s| !s.is_empty()).collect();
    // Nejhlubší první (víc lomítek = hlouběji vnořené).
    items.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));

    for item in &items {
        run(
            CODESIGN,
            &["--force", "--timestamp=none", "--sign", sha1, "--keychain", keychain.to_str().unwrap(), item],
        )
        .await
        .with_context(|| format!("podpis vnořeného {item}"))?;
    }

    // Hlavní app s entitlements.
    run(
        CODESIGN,
        &[
            "--force", "--timestamp=none", "--sign", sha1,
            "--entitlements", ent_path.to_str().unwrap(),
            "--keychain", keychain.to_str().unwrap(),
            app_dir.to_str().unwrap(),
        ],
    )
    .await
    .context("podpis hlavní app")?;

    // Ověř podpis lokálně (chytne nepodepsaný nested kód dřív než zařízení).
    run(CODESIGN, &["--verify", "--deep", "--strict", app_dir.to_str().unwrap()])
        .await
        .context("ověření podpisu (--verify --deep --strict)")?;
    tracing::info!("podpis ověřen (--verify --deep --strict OK)");
    Ok(())
}

/// Přepíše CFBundleIdentifier v Info.plist (zachová formát binary/xml).
async fn rewrite_bundle_id(info_plist: &Path, new_id: &str) -> Result<()> {
    let bytes = tokio::fs::read(info_plist).await?;
    let mut val: plist::Value = plist::from_bytes(&bytes)?;
    let dict = val.as_dictionary_mut().ok_or_else(|| anyhow!("Info.plist není dict"))?;
    dict.insert("CFBundleIdentifier".into(), plist::Value::String(new_id.to_string()));
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &val)?;
    tokio::fs::write(info_plist, out).await?;
    Ok(())
}

async fn first_app_bundle(payload: &Path) -> Result<PathBuf> {
    let mut rd = tokio::fs::read_dir(payload).await.context("Payload chybí")?;
    while let Some(e) = rd.next_entry().await? {
        let p = e.path();
        if p.extension().map(|x| x == "app").unwrap_or(false) {
            return Ok(p);
        }
    }
    Err(anyhow!("v Payload není .app"))
}

/// Vytáhne `<Entitlements>` dict z provisioning profilu jako XML plist.
fn extract_entitlements(profile: &[u8]) -> Option<Vec<u8>> {
    let start = find(profile, b"<?xml")?;
    let end = find(&profile[start..], b"</plist>")? + start + b"</plist>".len();
    let plist_bytes = &profile[start..end];
    let val: plist::Value = plist::from_bytes(plist_bytes).ok()?;
    let dict = val.as_dictionary()?;
    let ent = dict.get("Entitlements")?;
    let mut out = Vec::new();
    plist::to_writer_xml(&mut out, ent).ok()?;
    Some(out)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Dočasný keychain s importovanou podpisovou identitou.
struct Keychain {
    path: PathBuf,
    prev_list: Vec<String>,
}

impl Keychain {
    async fn create(st: &AppState, identity: &Identity) -> Result<Keychain> {
        let path = st.cfg.data_dir.join("signing").join("homesign.keychain-db");
        let _ = tokio::fs::remove_file(&path).await;
        let pass = "homesign";

        run(SECURITY, &["create-keychain", "-p", pass, path.to_str().unwrap()]).await?;
        run(SECURITY, &["unlock-keychain", "-p", pass, path.to_str().unwrap()]).await?;
        run(SECURITY, &["set-keychain-settings", path.to_str().unwrap()]).await?;

        // Postav p12 z cert+klíč a naimportuj.
        let dir = st.cfg.data_dir.join("signing");
        let cert_pem = dir.join("cert.pem");
        run(
            OPENSSL,
            &["x509", "-inform", "DER", "-in", identity.cert_der_path.to_str().unwrap(),
              "-out", cert_pem.to_str().unwrap()],
        )
        .await?;
        let p12 = dir.join("identity.p12");
        run(
            OPENSSL,
            &["pkcs12", "-export", "-inkey", identity.key_pem_path.to_str().unwrap(),
              "-in", cert_pem.to_str().unwrap(), "-out", p12.to_str().unwrap(),
              "-passout", "pass:homesign", "-name", "homesign"],
        )
        .await?;
        run(
            SECURITY,
            &["import", p12.to_str().unwrap(), "-k", path.to_str().unwrap(),
              "-P", "homesign", "-A", "-T", CODESIGN],
        )
        .await?;
        // Povol codesignu přístup ke klíči bez UI promptu.
        let _ = run(
            SECURITY,
            &["set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s",
              "-k", pass, path.to_str().unwrap()],
        )
        .await;

        // Přidej keychain do search listu, ať ho codesign najde.
        let prev = output(SECURITY, &["list-keychains", "-d", "user"]).await.unwrap_or_default();
        let prev_list: Vec<String> = prev
            .lines()
            .map(|l| l.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut new_list = prev_list.clone();
        new_list.insert(0, path.to_string_lossy().to_string());
        let mut args = vec!["list-keychains", "-d", "user", "-s"];
        for k in &new_list {
            args.push(k.as_str());
        }
        run(SECURITY, &args).await?;

        Ok(Keychain { path, prev_list })
    }

    async fn cleanup(&self) {
        // Obnov původní search list.
        let mut args = vec!["list-keychains".to_string(), "-d".into(), "user".into(), "-s".into()];
        args.extend(self.prev_list.iter().cloned());
        let argrefs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let _ = run(SECURITY, &argrefs).await;
        let _ = run(SECURITY, &["delete-keychain", self.path.to_str().unwrap()]).await;
    }
}

// ------------------------------------------------------------- proces util

async fn run(bin: &str, args: &[&str]) -> Result<()> {
    let out = tokio::process::Command::new(bin).args(args).output().await
        .with_context(|| format!("spuštění {bin}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "{bin} selhal: {}",
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        ));
    }
    Ok(())
}

async fn run_in(dir: &Path, bin: &str, args: &[&str]) -> Result<()> {
    let out = tokio::process::Command::new(bin).current_dir(dir).args(args).output().await
        .with_context(|| format!("spuštění {bin}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "{bin} selhal: {}",
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        ));
    }
    Ok(())
}

async fn output(bin: &str, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(bin).args(args).output().await
        .with_context(|| format!("spuštění {bin}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "{bin} selhal: {}",
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
