//! Chromium password/cookie decryption for macOS
//!
//! Uses AES-128-CBC with fixed IV, key from macOS Keychain.

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
        .args(&[
            "find-generic-password",
            "-s", service,
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
    
    let password = String::from_utf8(output.stdout)?
        .trim()
        .to_string();
    
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
    // Check for "v10" or "v11" prefix (Chromium version markers)
    if encrypted.len() < 3 {
        return Err(anyhow!("Encrypted data too short"));
    }
    
    // Skip the version prefix (first 3 bytes: "v10" or "v11")
    let ciphertext = &encrypted[3..];
    
    if ciphertext.is_empty() {
        return Ok(String::new());
    }
    
    // Decrypt using AES-128-CBC
    let mut buf = ciphertext.to_vec();
    let decrypted = Aes128CbcDec::new(key.into(), &CHROMIUM_MAC_IV.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow!("Decryption failed: {:?}", e))?;
    
    String::from_utf8(decrypted.to_vec())
        .map_err(|e| anyhow!("UTF-8 conversion failed: {}", e))
}

/// Check if data is encrypted (has Chromium version prefix)
pub fn is_encrypted(data: &[u8]) -> bool {
    data.len() >= 3 && (data.starts_with(b"v10") || data.starts_with(b"v11"))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_is_encrypted() {
        assert!(is_encrypted(b"v10encrypted_data"));
        assert!(is_encrypted(b"v11encrypted_data"));
        assert!(!is_encrypted(b"plain_text"));
        assert!(!is_encrypted(b"v1")); // Too short
    }
}
