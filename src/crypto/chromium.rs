//! Chromium password/cookie decryption for macOS
//!
//! macOS Chromium uses:
//! - v10 prefix: AES-128-CBC with fixed IV (space characters)
//! - Older versions: may use different schemes
//!
//! Key is derived from Keychain password using PBKDF2-HMAC-SHA1

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use anyhow::{anyhow, Result};
use std::process::Command;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Fixed IV used by Chromium on macOS (16 bytes of space character 0x20)
const CHROMIUM_MAC_IV: [u8; 16] = [0x20; 16];

/// Get Chromium master key from macOS Keychain
/// Returns 16-byte key for AES-128 decryption
pub fn get_chromium_master_key(browser: &str) -> Result<Vec<u8>> {
    // Service name varies by browser
    let service = match browser.to_lowercase().as_str() {
        "chrome" | "google chrome" => "Chrome Safe Storage",
        "edge" | "microsoft edge" => "Microsoft Edge Safe Storage",
        "brave" => "Brave Safe Storage",
        "opera" => "Opera Safe Storage",
        "vivaldi" => "Vivaldi Safe Storage",
        "arc" => "Arc Safe Storage",
        _ => "Chrome Safe Storage",
    };

    // Use security command to get password from Keychain
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            service,
            "-w", // Output password only
        ])
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "Failed to get master key from Keychain. Browser: {}. Error: {}",
            browser,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let password = String::from_utf8(output.stdout)?.trim().to_string();

    // Derive 16-byte key using PBKDF2
    // Chromium uses: PBKDF2_HMAC_SHA1(password, salt="saltysalt", iterations=1003, key_length=16)
    let key = derive_chromium_key(&password)?;

    Ok(key)
}

/// Derive Chromium encryption key using PBKDF2
fn derive_chromium_key(password: &str) -> Result<Vec<u8>> {
    use pbkdf2::pbkdf2_hmac;
    use sha1::Sha1;

    const SALT: &[u8] = b"saltysalt";
    const ITERATIONS: u32 = 1003;
    const KEY_LENGTH: usize = 16;

    let mut key = vec![0u8; KEY_LENGTH];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), SALT, ITERATIONS, &mut key);

    Ok(key)
}

/// Decrypt Chromium encrypted data (password, cookie value, etc.)
/// Returns decrypted plaintext
pub fn decrypt_chromium_data(key: &[u8], encrypted: &[u8]) -> Result<String> {
    // Check for "v10" prefix (macOS Chromium marker)
    if encrypted.len() < 3 {
        return Err(anyhow!("Encrypted data too short"));
    }

    // v10 = macOS encryption scheme
    if !encrypted.starts_with(b"v10") {
        // Not encrypted or unknown scheme - return as-is if valid UTF-8
        return String::from_utf8(encrypted.to_vec())
            .map_err(|_| anyhow!("Unknown encryption scheme"));
    }

    // Skip the "v10" prefix (3 bytes)
    let ciphertext = &encrypted[3..];

    if ciphertext.is_empty() {
        return Ok(String::new());
    }

    // macOS uses AES-128-CBC with fixed IV
    let mut buf = ciphertext.to_vec();

    // Try AES-128-CBC decryption
    match Aes128CbcDec::new(key.into(), &CHROMIUM_MAC_IV.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
    {
        Ok(decrypted) => {
            // Filter out non-printable characters that might be padding artifacts
            let clean: Vec<u8> = decrypted
                .iter()
                .take_while(|&&b| b >= 0x20 || b == 0x09 || b == 0x0A || b == 0x0D)
                .cloned()
                .collect();

            String::from_utf8(clean)
                .or_else(|_| String::from_utf8(decrypted.to_vec()))
                .map_err(|e| anyhow!("UTF-8 conversion failed: {}", e))
        }
        Err(e) => {
            // Decryption failed - might be corrupted or different encryption
            Err(anyhow!(
                "AES-CBC decryption failed: {:?}. Ciphertext len: {}",
                e,
                ciphertext.len()
            ))
        }
    }
}

/// Check if data is encrypted (has Chromium version prefix)
pub fn is_encrypted(data: &[u8]) -> bool {
    data.len() >= 3 && data.starts_with(b"v10")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_encrypted() {
        assert!(is_encrypted(b"v10encrypted_data"));
        assert!(!is_encrypted(b"v11encrypted_data")); // v11 not used on macOS
        assert!(!is_encrypted(b"plain_text"));
        assert!(!is_encrypted(b"v1")); // Too short
    }

    #[test]
    fn test_derive_key() {
        let key = derive_chromium_key("test_password").unwrap();
        assert_eq!(key.len(), 16);
    }
}
