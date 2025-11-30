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
    BraveNightly,
    Chrome,
    FirefoxNightly,
}

impl BrowserType {
    pub fn name(&self) -> &'static str {
        match self {
            BrowserType::Waterfox => "Waterfox",
            BrowserType::Safari => "Safari",
            BrowserType::Brave => "Brave",
            BrowserType::BraveNightly => "Brave Nightly",
            BrowserType::Chrome => "Chrome",
            BrowserType::FirefoxNightly => "Firefox Nightly",
        }
    }
}

pub trait BrowserAdapter: Send + Sync {
    fn browser_type(&self) -> BrowserType;
    fn detect_bookmark_path(&self) -> Result<PathBuf>;
    fn read_bookmarks(&self) -> Result<Vec<Bookmark>>;
    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()>;
    fn backup_bookmarks(&self) -> Result<PathBuf>;
    fn validate_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<bool>;
}

pub fn get_all_adapters() -> Vec<Box<dyn BrowserAdapter + Send + Sync>> {
    vec![
        Box::new(WaterfoxAdapter),
        Box::new(SafariAdapter),
        Box::new(BraveAdapter),
        Box::new(BraveNightlyAdapter),
        Box::new(ChromeAdapter),
        Box::new(FirefoxNightlyAdapter),
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
        read_firefox_bookmarks(&path)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let path = self.detect_bookmark_path()?;
        write_firefox_bookmarks(&path, bookmarks)
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
            let mut data = Vec::new();
            plist::to_writer_xml(&mut data, &plist_value)?;
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

// Brave Nightly Adapter
pub struct BraveNightlyAdapter;

impl BrowserAdapter for BraveNightlyAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::BraveNightly
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{}/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks",
                home
            ));
            
            if !path.exists() {
                anyhow::bail!("Brave Nightly bookmarks file not found");
            }
            
            debug!("Found Brave Nightly bookmarks at: {:?}", path);
            Ok(path)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Brave Nightly detection not implemented for this platform")
        }
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let path = self.detect_bookmark_path()?;
        let data = std::fs::read_to_string(&path)?;
        let json: serde_json::Value = serde_json::from_str(&data)?;
        
        let bookmarks = parse_chromium_bookmarks(&json)?;
        debug!("Read {} bookmarks from Brave Nightly", bookmarks.len());
        Ok(bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let path = self.detect_bookmark_path()?;
        self.backup_bookmarks()?;
        
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        std::fs::write(&path, data)?;
        
        debug!("Wrote {} bookmarks to Brave Nightly", bookmarks.len());
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

// Chrome Adapter
pub struct ChromeAdapter;

impl BrowserAdapter for ChromeAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Chrome
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{}/Library/Application Support/Google/Chrome/Default/Bookmarks",
                home
            ));
            
            if !path.exists() {
                anyhow::bail!("Chrome bookmarks file not found");
            }
            
            debug!("Found Chrome bookmarks at: {:?}", path);
            Ok(path)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Chrome detection not implemented for this platform")
        }
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let path = self.detect_bookmark_path()?;
        let data = std::fs::read_to_string(&path)?;
        let json: serde_json::Value = serde_json::from_str(&data)?;
        
        let bookmarks = parse_chromium_bookmarks(&json)?;
        debug!("Read {} bookmarks from Chrome", bookmarks.len());
        Ok(bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let path = self.detect_bookmark_path()?;
        self.backup_bookmarks()?;
        
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        std::fs::write(&path, data)?;
        
        debug!("Wrote {} bookmarks to Chrome", bookmarks.len());
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

// Firefox Nightly Adapter
pub struct FirefoxNightlyAdapter;

impl BrowserAdapter for FirefoxNightlyAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::FirefoxNightly
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
        read_firefox_bookmarks(&path)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let path = self.detect_bookmark_path()?;
        write_firefox_bookmarks(&path, bookmarks)
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
        // Parse all root folders
        for (_key, root) in roots.as_object().unwrap_or(&serde_json::Map::new()) {
            parse_chromium_node_recursive(root, &mut bookmarks)?;
        }
    }
    
    Ok(bookmarks)
}

fn parse_chromium_node_recursive(node: &serde_json::Value, bookmarks: &mut Vec<Bookmark>) -> Result<()> {
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            let is_folder = child.get("type").and_then(|v| v.as_str()) == Some("folder");
            
            let bookmark = Bookmark {
                id: child.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                title: child.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: child.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                folder: is_folder,
                children: vec![],
                date_added: child.get("date_added").and_then(|v| v.as_i64()),
                date_modified: child.get("date_modified").and_then(|v| v.as_i64()),
            };
            
            // Only add actual bookmarks (URLs), not folders
            if !is_folder && bookmark.url.is_some() {
                bookmarks.push(bookmark);
            }
            
            // Recursively parse children if it's a folder
            if is_folder {
                parse_chromium_node_recursive(child, bookmarks)?;
            }
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

// Firefox SQLite helper functions
fn read_firefox_bookmarks(db_path: &std::path::Path) -> Result<Vec<Bookmark>> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    let mut bookmarks = Vec::new();
    
    let mut stmt = conn.prepare(
        "SELECT b.id, b.title, p.url, b.dateAdded, b.lastModified, b.type
         FROM moz_bookmarks b
         LEFT JOIN moz_places p ON b.fk = p.id
         WHERE b.type IN (1, 2)
         ORDER BY b.position"
    )?;
    
    let bookmark_iter = stmt.query_map([], |row| {
        let bookmark_type: i32 = row.get(5)?;
        Ok(Bookmark {
            id: row.get::<_, i64>(0)?.to_string(),
            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            url: row.get::<_, Option<String>>(2)?,
            folder: bookmark_type == 2,
            children: vec![],
            date_added: row.get::<_, Option<i64>>(3)?,
            date_modified: row.get::<_, Option<i64>>(4)?,
        })
    })?;
    
    for bookmark in bookmark_iter {
        bookmarks.push(bookmark?);
    }
    
    debug!("Read {} bookmarks from Firefox database", bookmarks.len());
    Ok(bookmarks)
}

fn write_firefox_bookmarks(db_path: &std::path::Path, bookmarks: &[Bookmark]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    // Start transaction
    conn.execute("BEGIN TRANSACTION", [])?;
    
    // Clear existing bookmarks (except special folders)
    conn.execute(
        "DELETE FROM moz_bookmarks WHERE type = 1 AND parent NOT IN (1, 2, 3, 4)",
        [],
    )?;
    
    // Insert new bookmarks
    for bookmark in bookmarks {
        if bookmark.folder {
            continue; // Skip folders for now
        }
        
        if let Some(url) = &bookmark.url {
            // First, ensure the URL exists in moz_places
            conn.execute(
                "INSERT OR IGNORE INTO moz_places (url, title, rev_host, hidden, typed, frecency)
                 VALUES (?1, ?2, '', 0, 0, -1)",
                [url, &bookmark.title],
            )?;
            
            // Get the place_id
            let place_id: i64 = conn.query_row(
                "SELECT id FROM moz_places WHERE url = ?1",
                [url],
                |row| row.get(0),
            )?;
            
            // Insert bookmark
            conn.execute(
                "INSERT INTO moz_bookmarks (type, fk, parent, position, title, dateAdded, lastModified)
                 VALUES (1, ?1, 3, 0, ?2, ?3, ?4)",
                rusqlite::params![
                    place_id,
                    &bookmark.title,
                    bookmark.date_added.unwrap_or(0),
                    bookmark.date_modified.unwrap_or(0),
                ],
            )?;
        }
    }
    
    // Commit transaction
    conn.execute("COMMIT", [])?;
    
    debug!("Wrote {} bookmarks to Firefox database", bookmarks.len());
    Ok(())
}
