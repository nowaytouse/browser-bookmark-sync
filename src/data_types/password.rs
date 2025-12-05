//! Password data type and extraction
//!
//! Supports Chromium and Firefox browsers.

use crate::crypto;
use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Password entry from browser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Password {
    pub url: String,
    pub username: String,
    pub password: String,
    pub created_at: i64,
    pub browser: String,
}

/// Extract passwords from Chromium-based browser
pub fn extract_chromium_passwords(db_path: &Path, browser: &str) -> Result<Vec<Password>> {
    // Get master key from Keychain
    let master_key = crypto::get_chromium_master_key(browser)?;
    
    // Open database (use copy, not original)
    let conn = Connection::open(db_path)?;
    
    let mut stmt = conn.prepare(
        "SELECT origin_url, username_value, password_value, date_created FROM logins"
    )?;
    
    let mut passwords = Vec::new();
    
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    
    for row in rows {
        let (url, username, encrypted_pwd, created_at) = row?;
        
        let password = if crypto::is_encrypted(&encrypted_pwd) {
            crypto::decrypt_chromium_data(&master_key, &encrypted_pwd)
                .unwrap_or_else(|_| "[decryption failed]".to_string())
        } else {
            String::from_utf8_lossy(&encrypted_pwd).to_string()
        };
        
        passwords.push(Password {
            url,
            username,
            password,
            created_at,
            browser: browser.to_string(),
        });
    }
    
    // Sort by creation date descending
    passwords.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    
    Ok(passwords)
}

/// Extract passwords from Firefox
pub fn extract_firefox_passwords(profile_path: &Path) -> Result<Vec<Password>> {
    let logins_file = profile_path.join("logins.json");
    
    if !logins_file.exists() {
        return Err(anyhow!("logins.json not found"));
    }
    
    let content = std::fs::read_to_string(&logins_file)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;
    
    let mut passwords = Vec::new();
    
    if let Some(logins) = json["logins"].as_array() {
        for login in logins {
            // Firefox passwords are encrypted with NSS - complex to decrypt
            // For now, just note they exist
            passwords.push(Password {
                url: login["hostname"].as_str().unwrap_or("").to_string(),
                username: "[encrypted]".to_string(),
                password: "[encrypted - NSS]".to_string(),
                created_at: login["timeCreated"].as_i64().unwrap_or(0) / 1000,
                browser: "Firefox".to_string(),
            });
        }
    }
    
    Ok(passwords)
}

/// Export passwords to CSV format
pub fn export_to_csv(passwords: &[Password], output_path: &Path) -> Result<()> {
    use std::io::Write;
    
    let mut file = std::fs::File::create(output_path)?;
    
    // Header
    writeln!(file, "url,username,password,browser,created_at")?;
    
    for pwd in passwords {
        writeln!(
            file,
            "\"{}\",\"{}\",\"{}\",\"{}\",{}",
            pwd.url.replace('"', "\"\""),
            pwd.username.replace('"', "\"\""),
            pwd.password.replace('"', "\"\""),
            pwd.browser,
            pwd.created_at
        )?;
    }
    
    Ok(())
}

/// Export passwords to JSON format
pub fn export_to_json(passwords: &[Password], output_path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(passwords)?;
    std::fs::write(output_path, json)?;
    Ok(())
}
