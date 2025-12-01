use anyhow::Result;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tracing::{info, warn};

use crate::browsers::BrowserType;

/// Force close browsers before sync to ensure clean state
pub fn close_browsers(browsers: &[BrowserType], force: bool) -> Result<()> {
    info!("🔒 Closing browsers before sync...");
    
    for browser in browsers {
        close_browser(browser, force)?;
    }
    
    // Wait for browsers to fully close
    thread::sleep(Duration::from_secs(2));
    info!("✅ All browsers closed");
    
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
    };
    
    if !force {
        // Try graceful close with AppleScript
        let script = format!("tell application \"{}\" to quit", app_name);
        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output();
        
        if output.is_ok() {
            info!("  ✅ {} closed gracefully", app_name);
            return Ok(());
        } else {
            warn!("  ⚠️  {} graceful close failed, trying force kill", app_name);
        }
    }
    
    // Force kill
    let output = Command::new("killall")
        .arg("-9")
        .arg(process_name)
        .output();
    
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
                _ => None,
            }
        })
        .collect()
}
