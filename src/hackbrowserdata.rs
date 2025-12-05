//! HackBrowserData integration module
//!
//! Uses the compiled Go binary for complete browser data extraction.
//! Supports: passwords, cookies, history, bookmarks, downloads, localStorage, extensions

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Get the path to hack-browser-data binary
fn get_binary_path() -> Result<PathBuf> {
    // Try relative path first (in bin/ directory)
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    
    if let Some(dir) = exe_dir {
        let binary = dir.join("hack-browser-data");
        if binary.exists() {
            return Ok(binary);
        }
        
        // Try parent directory
        if let Some(parent) = dir.parent() {
            let binary = parent.join("bin").join("hack-browser-data");
            if binary.exists() {
                return Ok(binary);
            }
        }
    }
    
    // Try workspace bin
    let workspace_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("bin")
        .join("hack-browser-data");
    if workspace_bin.exists() {
        return Ok(workspace_bin);
    }
    
    Err(anyhow!("hack-browser-data binary not found. Please ensure it exists in bin/ directory"))
}

/// Browser type for HackBrowserData
#[derive(Debug, Clone, Copy)]
pub enum HackBrowser {
    All,
    Chrome,
    ChromeBeta,
    Edge,
    Brave,
    Opera,
    OperaGX,
    Vivaldi,
    Firefox,
    Arc,
}

impl HackBrowser {
    fn to_arg(&self) -> &str {
        match self {
            HackBrowser::All => "all",
            HackBrowser::Chrome => "chrome",
            HackBrowser::ChromeBeta => "chrome-beta",
            HackBrowser::Edge => "edge",
            HackBrowser::Brave => "brave",
            HackBrowser::Opera => "opera",
            HackBrowser::OperaGX => "opera-gx",
            HackBrowser::Vivaldi => "vivaldi",
            HackBrowser::Firefox => "firefox",
            HackBrowser::Arc => "arc",
        }
    }
    
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "chrome" => HackBrowser::Chrome,
            "chrome-beta" => HackBrowser::ChromeBeta,
            "edge" => HackBrowser::Edge,
            "brave" => HackBrowser::Brave,
            "opera" => HackBrowser::Opera,
            "opera-gx" => HackBrowser::OperaGX,
            "vivaldi" => HackBrowser::Vivaldi,
            "firefox" => HackBrowser::Firefox,
            "arc" => HackBrowser::Arc,
            _ => HackBrowser::All,
        }
    }
}

/// Export format
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    CSV,
    JSON,
}

impl ExportFormat {
    fn to_arg(&self) -> &str {
        match self {
            ExportFormat::CSV => "csv",
            ExportFormat::JSON => "json",
        }
    }
}

/// Run hack-browser-data to export browser data
pub fn export_browser_data(
    browser: HackBrowser,
    output_dir: &Path,
    format: ExportFormat,
    compress: bool,
) -> Result<PathBuf> {
    let binary = get_binary_path()?;
    
    std::fs::create_dir_all(output_dir)?;
    
    let mut cmd = Command::new(&binary);
    cmd.arg("-b").arg(browser.to_arg())
       .arg("-f").arg(format.to_arg())
       .arg("--dir").arg(output_dir);
    
    if compress {
        cmd.arg("--zip");
    }
    
    let output = cmd.output()?;
    
    if !output.status.success() {
        return Err(anyhow!(
            "hack-browser-data failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    
    Ok(output_dir.to_path_buf())
}

/// Password entry from HackBrowserData JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HackPassword {
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(rename = "UserName")]
    pub username: String,
    #[serde(rename = "Password")]
    pub password: String,
    #[serde(rename = "CreateDate")]
    pub create_date: String,
}

/// Cookie entry from HackBrowserData JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HackCookie {
    #[serde(rename = "Host")]
    pub host: String,
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "KeyName")]
    pub name: String,
    #[serde(rename = "Value")]
    pub value: String,
    #[serde(rename = "IsSecure")]
    pub is_secure: bool,
    #[serde(rename = "IsHTTPOnly")]
    pub is_http_only: bool,
    #[serde(rename = "ExpireDate")]
    pub expire_date: String,
}

/// Load passwords from exported JSON file
pub fn load_passwords(json_path: &Path) -> Result<Vec<HackPassword>> {
    let content = std::fs::read_to_string(json_path)?;
    let passwords: Vec<HackPassword> = serde_json::from_str(&content)?;
    Ok(passwords)
}

/// Load cookies from exported JSON file
pub fn load_cookies(json_path: &Path) -> Result<Vec<HackCookie>> {
    let content = std::fs::read_to_string(json_path)?;
    let cookies: Vec<HackCookie> = serde_json::from_str(&content)?;
    Ok(cookies)
}

/// Export and return passwords for a browser
pub fn get_passwords(browser: HackBrowser) -> Result<Vec<HackPassword>> {
    let temp_dir = std::env::temp_dir().join("browser-sync-hack");
    std::fs::create_dir_all(&temp_dir)?;
    
    export_browser_data(browser, &temp_dir, ExportFormat::JSON, false)?;
    
    // Find password file
    let password_file = std::fs::read_dir(&temp_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().contains("password"))
        .map(|e| e.path());
    
    if let Some(file) = password_file {
        load_passwords(&file)
    } else {
        Ok(vec![])
    }
}

/// Export and return cookies for a browser
pub fn get_cookies(browser: HackBrowser) -> Result<Vec<HackCookie>> {
    let temp_dir = std::env::temp_dir().join("browser-sync-hack");
    std::fs::create_dir_all(&temp_dir)?;
    
    export_browser_data(browser, &temp_dir, ExportFormat::JSON, false)?;
    
    // Find cookie file
    let cookie_file = std::fs::read_dir(&temp_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().contains("cookie"))
        .map(|e| e.path());
    
    if let Some(file) = cookie_file {
        load_cookies(&file)
    } else {
        Ok(vec![])
    }
}
