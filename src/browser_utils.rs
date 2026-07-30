use anyhow::{Context, Result};
use blake3::Hasher;
use chrono::Utc;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tracing::{info, warn};

use crate::browsers::BrowserType;
use crate::data_types::models::{BrowserProfile, ProfileSnapshot};

/// Safely create a read-only snapshot copy of a browser profile
pub fn create_profile_snapshot(
    profile: &BrowserProfile,
    dest_dir: &Path,
) -> Result<ProfileSnapshot> {
    let snapshot_folder_name = format!(
        "{}_{}",
        profile.id.replace('/', "_"),
        Utc::now().timestamp_millis()
    );
    let snapshot_dir = dest_dir.join(&snapshot_folder_name);
    fs::create_dir_all(&snapshot_dir)
        .with_context(|| format!("Failed to create snapshot directory {snapshot_dir:?}"))?;

    let mut hashes = BTreeMap::new();

    // Copy relevant bookmark and DB files safely
    if profile.path.exists() {
        if profile.path.is_file() {
            // e.g. Safari Bookmarks.plist or single file
            if let Some(file_name) = profile.path.file_name() {
                let target_path = snapshot_dir.join(file_name);
                if fs::copy(&profile.path, &target_path).is_ok() {
                    if let Ok(hash) = calculate_file_hash(&target_path) {
                        hashes.insert(file_name.to_string_lossy().to_string(), hash);
                    }
                }
            }
        } else if profile.path.is_dir() {
            // Copy files like Bookmarks, places.sqlite, places.sqlite-wal, Local State, etc.
            let targets = [
                "Bookmarks",
                "Bookmarks.bak",
                "places.sqlite",
                "places.sqlite-wal",
                "places.sqlite-shm",
                "favicons.sqlite",
                "Local State",
                "Preferences",
            ];

            for entry in targets {
                let src_file = profile.path.join(entry);
                if src_file.exists() && src_file.is_file() {
                    let dst_file = snapshot_dir.join(entry);
                    if fs::copy(&src_file, &dst_file).is_ok() {
                        if let Ok(hash) = calculate_file_hash(&dst_file) {
                            hashes.insert(entry.to_string(), hash);
                        }
                    }
                }
            }
        }
    }

    Ok(ProfileSnapshot {
        original_profile: profile.clone(),
        snapshot_path: snapshot_dir,
        created_at: Utc::now(),
        blake3_hashes: hashes,
    })
}

fn calculate_file_hash(file_path: &Path) -> Result<String> {
    let mut file = fs::File::open(file_path)?;
    let mut hasher = Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Force close browsers before sync if user explicitly requests writeback
pub fn close_browsers(browsers: &[BrowserType], force: bool) -> Result<()> {
    info!("🔒 Closing browsers before sync...");

    for browser in browsers {
        close_browser(browser, force)?;
    }

    thread::sleep(Duration::from_secs(1));
    info!("✅ All target browsers closed");

    Ok(())
}

fn close_browser(browser: &BrowserType, force: bool) -> Result<()> {
    let (app_name, process_name) = match browser {
        BrowserType::Safari => ("Safari", "Safari"),
        BrowserType::Chrome => ("Google Chrome", "Google Chrome"),
        BrowserType::Brave => ("Brave Browser", "Brave Browser"),
        BrowserType::BraveNightly => ("Brave Browser Nightly", "Brave Browser Nightly"),
        BrowserType::Waterfox => ("Waterfox", "waterfox-bin"),
        BrowserType::FirefoxNightly => ("Firefox Nightly", "firefox"),
        BrowserType::LibreWolf => ("LibreWolf", "librewolf"),
    };

    if !force {
        let script = format!("tell application \"{app_name}\" to quit");
        let output = Command::new("osascript").arg("-e").arg(&script).output();

        if output.is_ok() {
            info!("  ✅ {} closed gracefully", app_name);
            return Ok(());
        }
    }

    let output = Command::new("killall").arg("-9").arg(process_name).output();

    match output {
        Ok(_) => {
            info!("  ✅ {} force killed", app_name);
        }
        Err(e) => {
            warn!("  ⚠️  Failed to kill {}: {}", app_name, e);
        }
    }

    Ok(())
}

/// Parse browser list from comma-separated string
pub fn parse_browser_list(browsers_str: &str) -> Vec<BrowserType> {
    browsers_str
        .split(',')
        .filter_map(|s| {
            let s = s.trim().to_lowercase();
            match s.as_str() {
                "safari" => Some(BrowserType::Safari),
                "chrome" => Some(BrowserType::Chrome),
                "brave" => Some(BrowserType::Brave),
                "brave-nightly" | "bravenightly" => Some(BrowserType::BraveNightly),
                "waterfox" => Some(BrowserType::Waterfox),
                "firefox-nightly" | "firefoxnightly" => Some(BrowserType::FirefoxNightly),
                "librewolf" => Some(BrowserType::LibreWolf),
                _ => None,
            }
        })
        .collect()
}
