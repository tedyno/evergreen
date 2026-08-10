//! Resign pipeline for macOS: provisioning via Developer Services (devportal)
//! + signing with the system tools (`openssl`, `security`, `codesign`).
//!
//! Homesign variant B runs only on macOS, so the most robust approach is to use
//! Apple's own `codesign` (it handles nested frameworks/extensions and CodeResources)
//! rather than reimplementing signing in Rust.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::apple::{devportal, XcodeAuth};
use crate::models::{Device, Ipa};
use crate::signer::SignedIpa;
use crate::state::AppState;

// Absolute paths — the GUI app has a stripped-down PATH.
const OPENSSL: &str = "/usr/bin/openssl";
const SECURITY: &str = "/usr/bin/security";
const CODESIGN: &str = "/usr/bin/codesign";
const UNZIP: &str = "/usr/bin/unzip";
const ZIP: &str = "/usr/bin/zip";

/// The whole resign flow: provisioning + signing. Returns the signed IPA.
pub async fn provision_and_sign(st: &AppState, ipa: &Ipa, device: &Device) -> Result<SignedIpa> {
    // Offline path: if we have cert+key+profile on disk (and the profile is valid),
    // we re-sign without a single Apple call (this also sidesteps throttle -22411).
    if let Some(signed) = try_offline_resign(st, ipa).await? {
        return Ok(signed);
    }

    let auth = st.apple.xcode_auth().await.context("získání Xcode auth")?;
    let team_id = devportal::first_team_id(&auth).await.context("listTeams")?;

    // 1) Register the device (idempotently).
    devportal::add_device(&auth, &team_id, &device.udid, &device.name)
        .await
        .context("registrace zařízení")?;

    // 2) Ensure the signing identity (cert + key), reused from disk.
    let identity = ensure_identity(st, &auth, &team_id).await.context("certifikát")?;

    // The original bundle id belongs to the app's developer (e.g. com.shapr3d.shapr) and
    // can't be registered on our account. We rewrite it to a unique one under our team —
    // deterministically (the same on refresh → upgrade, not a duplicate).
    let new_bundle_id = format!("{}.{}", ipa.bundle_id, team_id.to_lowercase());

    // 3) Ensure an App ID for the new bundle id.
    let app_id_id = devportal::ensure_app_id(&auth, &team_id, &new_bundle_id, &ipa.name)
        .await
        .context("App ID")?;

    // 4) Download the provisioning profile (and store it for future offline resign/refresh).
    let profile = devportal::download_profile(&auth, &team_id, &app_id_id)
        .await
        .context("provisioning profil")?;
    let _ = tokio::fs::write(signing_dir(st).join("profile.mobileprovision"), &profile).await;

    // 5) Sign the IPA (rewrites the bundle id in Info.plist).
    let signed_path = sign_ipa(st, ipa, &identity, &profile, &new_bundle_id).await?;

    let profile_expires = crate::signer::parse_profile_expiration_pub(&profile);
    Ok(SignedIpa {
        path: signed_path,
        signed_bundle_id: new_bundle_id,
        profile_expires,
    })
}

/// Offline resign from stored cert+key+profile — without calling Apple. None if
/// something is missing or the profile has expired.
async fn try_offline_resign(st: &AppState, ipa: &Ipa) -> Result<Option<SignedIpa>> {
    let dir = signing_dir(st);
    let key_pem = dir.join("key.pem");
    let cert_der = dir.join("cert.der");
    let profile_path = dir.join("profile.mobileprovision");
    if !key_pem.exists() || !cert_der.exists() || !profile_path.exists() {
        return Ok(None);
    }

    let profile = tokio::fs::read(&profile_path).await?;
    // Profile validity.
    let expires = crate::signer::parse_profile_expiration_pub(&profile);
    if expires.map(|e| e <= chrono::Utc::now()).unwrap_or(true) {
        tracing::info!("offline profil vypršel/neznámý — použiju online cestu");
        return Ok(None);
    }

    // new_bundle_id from application-identifier ("{TEAMID}.{bundleid}").
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

/// Extracts the bundle id from the profile (Entitlements → application-identifier without the team prefix).
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

/// Signing identity stored on disk (cert DER + private key PEM).
struct Identity {
    cert_der_path: PathBuf,
    key_pem_path: PathBuf,
    sha1: String,
}

fn signing_dir(st: &AppState) -> PathBuf {
    st.cfg.data_dir.join("signing")
}

/// Reuse cert+key from disk, otherwise generate a CSR and obtain a cert from the account.
async fn ensure_identity(st: &AppState, auth: &XcodeAuth, team_id: &str) -> Result<Identity> {
    let dir = signing_dir(st);
    tokio::fs::create_dir_all(&dir).await?;
    let key_pem = dir.join("key.pem");
    let cert_der = dir.join("cert.der");

    // 1) If we have a key, try to find an issued cert for it on the account (without wasting
    //    another cert from the free limit).
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

    // 2) Otherwise generate a new key + CSR and obtain a cert.
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

    // Pick the cert from the account whose public key matches our private one (reliable).
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

/// Modulus of the RSA private key (hex) — for matching against the cert.
async fn key_modulus(key_pem: &Path) -> Result<String> {
    let out = output(OPENSSL, &["rsa", "-in", key_pem.to_str().unwrap(), "-noout", "-modulus"]).await?;
    Ok(out.trim().to_string())
}

/// Modulus of the public key in the certificate (DER).
async fn cert_der_modulus(cert_der: &[u8]) -> Result<String> {
    // openssl reads DER from stdin.
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

/// Does the private key match the certificate?
async fn key_matches_cert(key_pem: &Path, cert_der: &Path) -> Result<bool> {
    let km = key_modulus(key_pem).await?;
    let der = tokio::fs::read(cert_der).await?;
    let cm = cert_der_modulus(&der).await?;
    Ok(km == cm)
}

/// SHA-1 fingerprint of the certificate (identity for `codesign -s`).
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

/// Serializes signing — the shared keychain and the global keychain search-list can't
/// tolerate concurrency (refresh scheduler vs. a user install job).
static SIGN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Unpacks the IPA, inserts the profile + entitlements and signs it with `codesign`; returns the .ipa.
async fn sign_ipa(
    st: &AppState,
    ipa: &Ipa,
    identity: &Identity,
    profile: &[u8],
    new_bundle_id: &str,
) -> Result<PathBuf> {
    let _guard = SIGN_LOCK.lock().await;
    let work = st.cfg.data_dir.join("signwork").join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&work).await?;

    // The actual work in a block, so we always clean up the work dir (even on error).
    let result = sign_ipa_inner(st, ipa, identity, profile, new_bundle_id, &work).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    result
}

async fn sign_ipa_inner(
    st: &AppState,
    ipa: &Ipa,
    identity: &Identity,
    profile: &[u8],
    new_bundle_id: &str,
    work: &Path,
) -> Result<PathBuf> {
    let work_s = work.to_string_lossy().to_string();

    // Unpack the IPA.
    run(UNZIP, &["-q", &ipa.path, "-d", &work_s]).await.context("unzip IPA")?;

    // Find Payload/*.app.
    let app_dir = first_app_bundle(&work.join("Payload")).await?;
    let app_s = app_dir.to_string_lossy().to_string();

    // Rewrite CFBundleIdentifier in Info.plist to the new bundle id.
    rewrite_bundle_id(&app_dir.join("Info.plist"), new_bundle_id).await
        .context("přepis bundle id v Info.plist")?;

    // Insert the provisioning profile.
    tokio::fs::write(app_dir.join("embedded.mobileprovision"), profile).await?;

    // Entitlements from the profile.
    let entitlements = extract_entitlements(profile)
        .ok_or_else(|| anyhow!("profil neobsahuje Entitlements"))?;
    let ent_path = work.join("entitlements.plist");
    tokio::fs::write(&ent_path, entitlements).await?;

    // Temporary keychain with the identity; sign the nested frameworks individually + verify.
    let keychain = Keychain::create(st, identity).await?;
    let sign_res = sign_all(&app_s, &ent_path, &identity.sha1, &keychain.path).await;
    keychain.cleanup().await;
    sign_res.context("codesign")?;

    // Pack back into the .ipa (zip from the work dir → paths "Payload/...").
    let out_ipa = st.cfg.ipa_dir().join(format!("{}-signed.ipa", ipa.id));
    let _ = tokio::fs::remove_file(&out_ipa).await;
    run_in(work, ZIP, &["-qr", &out_ipa.to_string_lossy(), "Payload"]).await.context("zip IPA")?;

    Ok(out_ipa)
}

/// Signs all nested frameworks/dylib/appex (from the deepest), then the app.
async fn sign_all(app_path: &str, ent_path: &Path, sha1: &str, keychain: &Path) -> Result<()> {
    let ent = ent_path.to_string_lossy().to_string();
    let kc = keychain.to_string_lossy().to_string();

    // Find the nested signable code.
    let out = output(
        "/usr/bin/find",
        &[
            app_path,
            "(", "-name", "*.framework", "-o", "-name", "*.dylib", "-o", "-name", "*.appex", ")",
        ],
    )
    .await
    .unwrap_or_default();

    let mut items: Vec<String> = out.lines().map(|s| s.to_string()).filter(|s| !s.is_empty()).collect();
    // Deepest first (more slashes = more deeply nested).
    items.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));

    for item in &items {
        run(
            CODESIGN,
            &["--force", "--timestamp=none", "--sign", sha1, "--keychain", &kc, item],
        )
        .await
        .with_context(|| format!("podpis vnořeného {item}"))?;
    }

    // The main app with entitlements.
    run(
        CODESIGN,
        &[
            "--force", "--timestamp=none", "--sign", sha1,
            "--entitlements", &ent, "--keychain", &kc, app_path,
        ],
    )
    .await
    .context("podpis hlavní app")?;

    // Verify the signature locally (catches unsigned nested code before the device does).
    run(CODESIGN, &["--verify", "--deep", "--strict", app_path])
        .await
        .context("ověření podpisu (--verify --deep --strict)")?;
    tracing::info!("podpis ověřen (--verify --deep --strict OK)");
    Ok(())
}

/// Rewrites CFBundleIdentifier in Info.plist (preserves the binary/xml format).
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

/// Extracts the `<Entitlements>` dict from the provisioning profile as an XML plist.
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

/// Temporary keychain with the imported signing identity.
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

        // Build a p12 from cert+key and import it.
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
        // Allow codesign to access the key without a UI prompt.
        let _ = run(
            SECURITY,
            &["set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s",
              "-k", pass, path.to_str().unwrap()],
        )
        .await;

        // Add the keychain to the search list so codesign finds it.
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
        // Restore the original search list.
        let mut args = vec!["list-keychains".to_string(), "-d".into(), "user".into(), "-s".into()];
        args.extend(self.prev_list.iter().cloned());
        let argrefs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let _ = run(SECURITY, &argrefs).await;
        let _ = run(SECURITY, &["delete-keychain", self.path.to_str().unwrap()]).await;
    }
}

// ------------------------------------------------------------- process util

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
