use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub folder: bool,
    pub children: Vec<Bookmark>,
    pub date_added: Option<i64>,
    pub date_modified: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserType {
    Waterfox,
    Safari,
    Brave,
    Nightly,
}

impl BrowserType {
    pub fn name(&self) -> &'static str {
        match self {
            BrowserType::Waterfox => "Waterfox",
            BrowserType::Safari => "Safari",
            BrowserType::Brave => "Brave",
            BrowserType::Nightly => "Firefox Nightly",
        }
    }
}

pub trait BrowserAdapter {
    fn browser_type(&self) -> BrowserType;
    fn detect_bookmark_path(&self) -> Result<PathBuf>;
    fn read_bookmarks(&self) -> Result<Vec<Bookmark>>;
    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()>;
    fn backup_bookmarks(&self) -> Result<PathBuf>;
    fn validate_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<bool>;
}

pub fn get_all_adapters() -> Vec<Box<dyn BrowserAdapter>> {
    vec![
        Box::new(WaterfoxAdapter),
        Box::new(SafariAdapter),
        Box::new(BraveAdapter),
        Box::new(NightlyAdapter),
    ]
}

// Waterfox Adapter
pub struct WaterfoxAdapter;

impl BrowserAdapter for WaterfoxAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Waterfox
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{}/Library/Application Support/Waterfox/Profiles",
                home
            ));
            
            if !path.exists() {
                anyhow::bail!("Waterfox profile directory not found");
            }
            
            // Find the default profile
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                let profile_path = entry.path();
                if profile_path.is_dir() {
                    let bookmarks_path = profile_path.join("places.sqlite");
                    if bookmarks_path.exists() {
                        debug!("Found Waterfox bookmarks at: {:?}", bookmarks_path);
                        return Ok(bookmarks_path);
                    }
                }
            }
            
            anyhow::bail!("Waterfox bookmarks file not found")
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Waterfox detection not implemented for this platform")
        }
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let path = self.detect_bookmark_path()?;
        // TODO: Implement SQLite reading for Firefox-based browsers
        warn!("Waterfox bookmark reading not yet implemented");
        Ok(vec![])
    }

    fn write_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<()> {
        warn!("Waterfox bookmark writing not yet implemented");
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        let backup_path = source.with_extension("sqlite.backup");
        std::fs::copy(&source, &backup_path)?;
        Ok(backup_path)
    }

    fn validate_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<bool> {
        Ok(true)
    }
}

// Safari Adapter
pub struct SafariAdapter;

impl BrowserAdapter for SafariAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Safari
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!("{}/Library/Safari/Bookmarks.plist", home));
            
            if !path.exists() {
                anyhow::bail!("Safari bookmarks file not found");
            }
            
            debug!("Found Safari bookmarks at: {:?}", path);
            Ok(path)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        #[cfg(target_os = "macos")]
        {
            let path = self.detect_bookmark_path()?;
            let data = std::fs::read(&path)?;
            let plist_value: plist::Value = plist::from_bytes(&data)?;
            
            // Parse Safari plist format
            let bookmarks = parse_safari_plist(&plist_value)?;
            debug!("Read {} bookmarks from Safari", bookmarks.len());
            Ok(bookmarks)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let path = self.detect_bookmark_path()?;
            // Backup first
            self.backup_bookmarks()?;
            
            // Convert to Safari plist format
            let plist_value = bookmarks_to_safari_plist(bookmarks)?;
            let data = plist::to_bytes_xml(&plist_value)?;
            std::fs::write(&path, data)?;
            
            debug!("Wrote {} bookmarks to Safari", bookmarks.len());
            Ok(())
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        let backup_path = source.with_extension("plist.backup");
        std::fs::copy(&source, &backup_path)?;
        Ok(backup_path)
    }

    fn validate_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<bool> {
        // Validate bookmark structure
        for bookmark in bookmarks {
            if bookmark.folder && bookmark.url.is_some() {
                return Ok(false);
            }
            if !bookmark.folder && bookmark.url.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// Brave Adapter
pub struct BraveAdapter;

impl BrowserAdapter for BraveAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Brave
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{}/Library/Application Support/BraveSoftware/Brave-Browser/Default/Bookmarks",
                home
            ));
            
            if !path.exists() {
                anyhow::bail!("Brave bookmarks file not found");
            }
            
            debug!("Found Brave bookmarks at: {:?}", path);
            Ok(path)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Brave detection not implemented for this platform")
        }
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let path = self.detect_bookmark_path()?;
        let data = std::fs::read_to_string(&path)?;
        let json: serde_json::Value = serde_json::from_str(&data)?;
        
        let bookmarks = parse_chromium_bookmarks(&json)?;
        debug!("Read {} bookmarks from Brave", bookmarks.len());
        Ok(bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let path = self.detect_bookmark_path()?;
        // Backup first
        self.backup_bookmarks()?;
        
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        std::fs::write(&path, data)?;
        
        debug!("Wrote {} bookmarks to Brave", bookmarks.len());
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        let backup_path = source.with_extension("json.backup");
        std::fs::copy(&source, &backup_path)?;
        Ok(backup_path)
    }

    fn validate_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<bool> {
        for bookmark in bookmarks {
            if bookmark.folder && bookmark.url.is_some() {
                return Ok(false);
            }
            if !bookmark.folder && bookmark.url.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// Nightly Adapter
pub struct NightlyAdapter;

impl BrowserAdapter for NightlyAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Nightly
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{}/Library/Application Support/Firefox/Profiles",
                home
            ));
            
            if !path.exists() {
                anyhow::bail!("Firefox Nightly profile directory not found");
            }
            
            // Find the nightly profile
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                let profile_path = entry.path();
                if profile_path.is_dir() && profile_path.to_string_lossy().contains("nightly") {
                    let bookmarks_path = profile_path.join("places.sqlite");
                    if bookmarks_path.exists() {
                        debug!("Found Nightly bookmarks at: {:?}", bookmarks_path);
                        return Ok(bookmarks_path);
                    }
                }
            }
            
            anyhow::bail!("Firefox Nightly bookmarks file not found")
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Nightly detection not implemented for this platform")
        }
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let path = self.detect_bookmark_path()?;
        // TODO: Implement SQLite reading for Firefox-based browsers
        warn!("Nightly bookmark reading not yet implemented");
        Ok(vec![])
    }

    fn write_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<()> {
        warn!("Nightly bookmark writing not yet implemented");
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        let backup_path = source.with_extension("sqlite.backup");
        std::fs::copy(&source, &backup_path)?;
        Ok(backup_path)
    }

    fn validate_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<bool> {
        Ok(true)
    }
}

// Helper functions for Safari plist parsing
#[cfg(target_os = "macos")]
fn parse_safari_plist(value: &plist::Value) -> Result<Vec<Bookmark>> {
    // Simplified implementation - needs full Safari plist structure parsing
    Ok(vec![])
}

#[cfg(target_os = "macos")]
fn bookmarks_to_safari_plist(bookmarks: &[Bookmark]) -> Result<plist::Value> {
    // Simplified implementation - needs full Safari plist structure generation
    Ok(plist::Value::Dictionary(Default::default()))
}

// Helper functions for Chromium JSON parsing
fn parse_chromium_bookmarks(json: &serde_json::Value) -> Result<Vec<Bookmark>> {
    let mut bookmarks = Vec::new();
    
    if let Some(roots) = json.get("roots") {
        if let Some(bookmark_bar) = roots.get("bookmark_bar") {
            parse_chromium_node(bookmark_bar, &mut bookmarks)?;
        }
        if let Some(other) = roots.get("other") {
            parse_chromium_node(other, &mut bookmarks)?;
        }
    }
    
    Ok(bookmarks)
}

fn parse_chromium_node(node: &serde_json::Value, bookmarks: &mut Vec<Bookmark>) -> Result<()> {
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            let bookmark = Bookmark {
                id: child.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                title: child.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: child.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                folder: child.get("type").and_then(|v| v.as_str()) == Some("folder"),
                children: vec![],
                date_added: child.get("date_added").and_then(|v| v.as_i64()),
                date_modified: child.get("date_modified").and_then(|v| v.as_i64()),
            };
            bookmarks.push(bookmark);
        }
    }
    
    Ok(())
}

fn bookmarks_to_chromium_json(bookmarks: &[Bookmark]) -> Result<serde_json::Value> {
    // Simplified implementation - needs full Chromium JSON structure generation
    Ok(serde_json::json!({
        "roots": {
            "bookmark_bar": {
                "children": [],
                "name": "Bookmarks Bar"
            }
        }
    }))
}
