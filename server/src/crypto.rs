//! Šifrování citlivých polí (Apple ID heslo, session tokeny) at rest.
//! AES-256-GCM, klíč z Config::master_key. Formát: base64(nonce[12] || ciphertext).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;

pub fn encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    Ok(STANDARD.encode(out))
}

pub fn decrypt(key: &[u8; 32], token: &str) -> anyhow::Result<String> {
    let raw = STANDARD.decode(token)?;
    if raw.len() < 12 {
        anyhow::bail!("ciphertext příliš krátký");
    }
    let (nonce_bytes, ct) = raw.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|e| anyhow::anyhow!("decrypt: {e}"))?;
    Ok(String::from_utf8(pt)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [7u8; 32];
        let ct = encrypt(&key, "tajné heslo").unwrap();
        assert_eq!(decrypt(&key, &ct).unwrap(), "tajné heslo");
    }

    #[test]
    fn wrong_key_fails() {
        let ct = encrypt(&[1u8; 32], "x").unwrap();
        assert!(decrypt(&[2u8; 32], &ct).is_err());
    }
}
