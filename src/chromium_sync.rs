//! Chromium Sync Detection Module
//!
//! Detects if browser sync is enabled and shows appropriate warnings.
//! Currently unused but kept for future sync conflict detection.

#![allow(dead_code)]

use anyhow::Result;
use std::path::Path;
use tracing::{info, warn};

/// Chromium sync status
#[derive(Debug)]
pub struct ChromiumSyncStatus {
    pub sync_enabled: bool,
    pub signed_in_email: Option<String>,
    pub browser_name: String,
}

impl ChromiumSyncStatus {
    /// Detect sync status from Chromium profile
    pub fn detect(profile_path: &Path, browser_name: &str) -> Result<Self> {
        let preferences_path = profile_path.join("Preferences");

        if !preferences_path.exists() {
            return Ok(Self {
                sync_enabled: false,
                signed_in_email: None,
                browser_name: browser_name.to_string(),
            });
        }

        let content = std::fs::read_to_string(&preferences_path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;

        // Check for Google account sign-in
        let email = json["account_info"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|acc| acc["email"].as_str())
            .map(|s| s.to_string());

        // Check if sync is enabled
        let sync_enabled = json["sync"]["has_setup_completed"]
            .as_bool()
            .unwrap_or(false)
            || email.is_some();

        Ok(Self {
            sync_enabled,
            signed_in_email: email,
            browser_name: browser_name.to_string(),
        })
    }

    /// Show warning if sync is enabled
    pub fn show_warning(&self) {
        if !self.sync_enabled {
            return;
        }

        warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        warn!("⚠️  {} Cloud Sync Detected!", self.browser_name);
        warn!("");
        if let Some(email) = &self.signed_in_email {
            warn!("   📧 Account: {}", email);
        }
        warn!("");
        warn!("   ⚠️  IMPORTANT:");
        warn!("   - Direct database modifications may conflict with cloud sync");
        warn!("   - Cloud data may overwrite local changes on next sync");
        warn!("   - Recommended: Use EXPORT to file, then IMPORT manually");
        warn!("");
        warn!("   💡 SAFE OPTION: bsync export -o bookmarks.html");
        warn!("      Then import the HTML file manually in browser settings");
        warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        warn!("");
    }

    /// Check if it's safe to write to database
    pub fn is_safe_to_write(&self) -> bool {
        !self.sync_enabled
    }
}

/// Detect sync status for all major Chromium browsers
pub fn detect_all_chromium_sync() -> Vec<ChromiumSyncStatus> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut results = Vec::new();

    let browsers = [
        ("Google/Chrome", "Chrome"),
        ("BraveSoftware/Brave-Browser", "Brave"),
        ("BraveSoftware/Brave-Browser-Nightly", "Brave Nightly"),
        ("Microsoft Edge", "Edge"),
        ("Arc/User Data", "Arc"),
        ("Vivaldi", "Vivaldi"),
    ];

    for (path, name) in browsers {
        let profile_path = format!("{}/Library/Application Support/{}/Default", home, path);
        let path = Path::new(&profile_path);

        if path.exists() {
            if let Ok(status) = ChromiumSyncStatus::detect(path, name) {
                if status.sync_enabled {
                    info!("🔄 {} sync detected", name);
                }
                results.push(status);
            }
        }
    }

    results
}

/// Show warnings for all synced browsers
pub fn show_all_sync_warnings() {
    for status in detect_all_chromium_sync() {
        status.show_warning();
    }
}
