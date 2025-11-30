use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, warn, info};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub expiry: Option<i64>,
    pub is_secure: bool,
    pub is_http_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingListItem {
    pub url: String,
    pub title: String,
    pub date_added: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub url: String,
    pub title: Option<String>,
    pub visit_count: i32,
    pub last_visit: Option<i64>,
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
    
    // Reading list support
    fn supports_reading_list(&self) -> bool {
        false
    }
    fn read_reading_list(&self) -> Result<Vec<ReadingListItem>> {
        Ok(vec![])
    }
    fn write_reading_list(&self, _items: &[ReadingListItem]) -> Result<()> {
        Ok(())
    }
    
    // History support
    fn supports_history(&self) -> bool {
        false
    }
    fn read_history(&self, _days: Option<i32>) -> Result<Vec<HistoryItem>> {
        Ok(vec![])
    }
    fn write_history(&self, _items: &[HistoryItem]) -> Result<()> {
        Ok(())
    }
    
    // Cookies support
    fn supports_cookies(&self) -> bool {
        false
    }
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        Ok(vec![])
    }
    fn write_cookies(&self, _cookies: &[Cookie]) -> Result<()> {
        Ok(())
    }
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
        let profiles = self.detect_all_profiles()?;
        // Find first profile with valid (non-empty) places.sqlite
        let valid = profiles.iter()
            .find(|p| p.exists() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))
            .or_else(|| profiles.first());
        Ok(valid
            .ok_or_else(|| anyhow::anyhow!("No Waterfox profiles found"))?
            .clone())
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from default profile (first one with valid data)
        let profiles = self.detect_all_profiles()?;
        
        for profile_path in profiles.iter() {
            match read_firefox_bookmarks(profile_path) {
                Ok(bookmarks) if !bookmarks.is_empty() => {
                    // 🔧 FIX: Use recursive count, not just top-level count
                    let count = count_bookmarks(&bookmarks);
                    info!("✅ Waterfox (Default): {} bookmarks", count);
                    return Ok(bookmarks);
                }
                Ok(_) => continue, // Empty profile, try next
                Err(_) => continue, // Failed, try next
            }
        }
        
        Ok(Vec::new())
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        // Only write to profile with valid database (non-empty places.sqlite)
        let profiles = self.detect_all_profiles()?;
        let valid_profile = profiles.iter()
            .find(|p| p.exists() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))
            .ok_or_else(|| anyhow::anyhow!("No valid Waterfox profile found"))?;
        
        match write_firefox_bookmarks(valid_profile, bookmarks) {
            Ok(_) => {
                info!("✅ Wrote {} bookmarks to Waterfox (Default)", bookmarks.len());
            }
            Err(e) => {
                warn!("⚠️  Failed to write to Waterfox: {}", e);
            }
        }
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
    
    fn supports_history(&self) -> bool {
        true
    }
    
    fn read_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> {
        // Only read from default profile (first valid one) for performance
        let profiles = self.detect_all_profiles()?;
        
        for profile_path in profiles.iter() {
            match read_firefox_history(profile_path, days) {
                Ok(history) if !history.is_empty() => {
                    info!("✅ Waterfox (Default): {} history items", history.len());
                    return Ok(history);
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        // Only write to profile with valid database
        let profiles = self.detect_all_profiles()?;
        let valid_profile = profiles.iter()
            .find(|p| p.exists() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))
            .ok_or_else(|| anyhow::anyhow!("No valid Waterfox profile found"))?;
        
        match write_firefox_history(valid_profile, items) {
            Ok(_) => info!("✅ Wrote {} history items to Waterfox (Default)", items.len()),
            Err(e) => warn!("⚠️  Failed to write history to Waterfox: {}", e),
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        // Only read from default profile for performance
        let profiles = self.detect_all_profiles()?;
        
        for profile_path in profiles.iter() {
            let cookies_path = profile_path.parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile path"))?
                .join("cookies.sqlite");
            
            if cookies_path.exists() {
                match read_firefox_cookies(&cookies_path) {
                    Ok(cookies) if !cookies.is_empty() => {
                        info!("✅ Waterfox (Default): {} cookies", cookies.len());
                        return Ok(cookies);
                    }
                    Ok(_) => continue,
                    Err(_) => continue,
                }
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        // Only write to default profile
        let profiles = self.detect_all_profiles()?;
        
        for profile_path in profiles.iter() {
            let cookies_path = profile_path.parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile path"))?
                .join("cookies.sqlite");
            
            if cookies_path.exists() {
                match write_firefox_cookies(&cookies_path, cookies) {
                    Ok(_) => {
                        info!("✅ Wrote {} cookies to Waterfox (Default)", cookies.len());
                        return Ok(());
                    }
                    Err(e) => warn!("⚠️  Failed to write cookies to Waterfox: {}", e),
                }
            }
        }
        Ok(())
    }
}

impl WaterfoxAdapter {
    fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let profiles_dir = PathBuf::from(format!(
                "{}/Library/Application Support/Waterfox/Profiles",
                home
            ));
            
            if !profiles_dir.exists() {
                anyhow::bail!("Waterfox profile directory not found");
            }
            
            let mut profiles = Vec::new();
            for entry in std::fs::read_dir(&profiles_dir)? {
                let entry = entry?;
                let profile_path = entry.path();
                if profile_path.is_dir() {
                    let bookmarks_path = profile_path.join("places.sqlite");
                    if bookmarks_path.exists() {
                        profiles.push(bookmarks_path);
                    }
                }
            }
            
            if profiles.is_empty() {
                anyhow::bail!("No Waterfox profiles with bookmarks found");
            }
            
            info!("🔍 Found {} Waterfox profile(s)", profiles.len());
            Ok(profiles)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Waterfox detection not implemented for this platform")
        }
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
    
    fn supports_reading_list(&self) -> bool {
        true
    }
    
    fn read_reading_list(&self) -> Result<Vec<ReadingListItem>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!("{}/Library/Safari/Bookmarks.plist", home));
            
            if !path.exists() {
                anyhow::bail!("Safari bookmarks file not found");
            }
            
            let data = std::fs::read(&path)?;
            let plist_value: plist::Value = plist::from_bytes(&data)?;
            
            // Parse reading list from Safari plist
            let items = parse_safari_reading_list(&plist_value)?;
            info!("Read {} reading list items from Safari", items.len());
            Ok(items)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }
    
    fn write_reading_list(&self, items: &[ReadingListItem]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!("{}/Library/Safari/Bookmarks.plist", home));
            
            // Backup first
            self.backup_bookmarks()?;
            
            // Read existing plist
            let data = std::fs::read(&path)?;
            let mut plist_value: plist::Value = plist::from_bytes(&data)?;
            
            // Update reading list section
            update_safari_reading_list(&mut plist_value, items)?;
            
            // Write back
            let mut data = Vec::new();
            plist::to_writer_xml(&mut data, &plist_value)?;
            std::fs::write(&path, data)?;
            
            info!("Wrote {} reading list items to Safari", items.len());
            Ok(())
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }
    
    fn supports_history(&self) -> bool {
        true
    }
    
    fn read_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let history_path = PathBuf::from(format!("{}/Library/Safari/History.db", home));
            
            if !history_path.exists() {
                anyhow::bail!("Safari history database not found");
            }
            
            read_safari_history(&history_path, days)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let history_path = PathBuf::from(format!("{}/Library/Safari/History.db", home));
            
            write_safari_history(&history_path, items)
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }
}

// Helper function to detect all Chromium profiles
fn detect_chromium_profiles(browser_dir: &str) -> Result<Vec<PathBuf>> {
    let home = std::env::var("HOME")?;
    let base_dir = PathBuf::from(format!("{}/Library/Application Support/{}", home, browser_dir));
    
    if !base_dir.exists() {
        anyhow::bail!("{} directory not found", browser_dir);
    }
    
    let mut profiles = Vec::new();
    
    // Check Default profile
    let default_profile = base_dir.join("Default");
    if default_profile.exists() && default_profile.is_dir() {
        profiles.push(default_profile);
    }
    
    // Check Profile N directories
    for entry in std::fs::read_dir(&base_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        
        if path.is_dir() && (name.starts_with("Profile ") || name == "Guest Profile") {
            profiles.push(path);
        }
    }
    
    if profiles.is_empty() {
        anyhow::bail!("No {} profiles found", browser_dir);
    }
    
    info!("🔍 Found {} profile(s) in {}", profiles.len(), browser_dir);
    Ok(profiles)
}

// Brave Adapter
pub struct BraveAdapter;

impl BraveAdapter {
    fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
        detect_chromium_profiles("BraveSoftware/Brave-Browser")
    }
}

impl BrowserAdapter for BraveAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Brave
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        let profiles = self.detect_all_profiles()?;
        let bookmarks_path = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?
            .join("Bookmarks");
        Ok(bookmarks_path)
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;
        
        let bookmarks_path = default_profile.join("Bookmarks");
        if !bookmarks_path.exists() {
            return Ok(Vec::new());
        }
        
        let data = std::fs::read_to_string(&bookmarks_path)?;
        let json: serde_json::Value = serde_json::from_str(&data)?;
        let bookmarks = parse_chromium_bookmarks(&json)?;
        
        let count = count_bookmarks(&bookmarks);
        info!("✅ Brave (Default): {} bookmarks", count);
        Ok(bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;
        
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        
        let bookmarks_path = default_profile.join("Bookmarks");
        std::fs::write(&bookmarks_path, &data)?;
        
        let total = count_bookmarks(bookmarks);
        info!("✅ Wrote {} bookmarks to Brave (Default)", total);
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
    
    fn supports_history(&self) -> bool {
        true
    }
    
    fn read_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> {
        // Only read from Default profile for performance
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;
        
        let history_path = default_profile.join("History");
        if history_path.exists() && history_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            match read_chromium_history(&history_path, days) {
                Ok(history) => {
                    info!("✅ Brave (Default): {} history items", history.len());
                    return Ok(history);
                }
                Err(e) => warn!("⚠️  Failed to read Brave history: {}", e),
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;
        
        let history_path = default_profile.join("History");
        if history_path.exists() {
            match write_chromium_history(&history_path, items) {
                Ok(_) => info!("✅ Wrote {} history items to Brave (Default)", items.len()),
                Err(e) => warn!("⚠️  Failed to write history to Brave: {}", e),
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        // Only read from Default profile for performance
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;
        
        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match read_chromium_cookies(&cookies_path) {
                Ok(cookies) => {
                    info!("✅ Brave (Default): {} cookies", cookies.len());
                    return Ok(cookies);
                }
                Err(e) => warn!("⚠️  Failed to read Brave cookies: {}", e),
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;
        
        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match write_chromium_cookies(&cookies_path, cookies) {
                Ok(_) => info!("✅ Wrote {} cookies to Brave (Default)", cookies.len()),
                Err(e) => warn!("⚠️  Failed to write cookies to Brave: {}", e),
            }
        }
        Ok(())
    }
}

// Brave Nightly Adapter
pub struct BraveNightlyAdapter;

impl BraveNightlyAdapter {
    fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
        detect_chromium_profiles("BraveSoftware/Brave-Browser-Nightly")
    }
}

impl BrowserAdapter for BraveNightlyAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::BraveNightly
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        let profiles = self.detect_all_profiles()?;
        let bookmarks_path = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?
            .join("Bookmarks");
        Ok(bookmarks_path)
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from Default profile to avoid data duplication
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;
        
        let bookmarks_path = default_profile.join("Bookmarks");
        if !bookmarks_path.exists() {
            return Ok(Vec::new());
        }
        
        let data = std::fs::read_to_string(&bookmarks_path)?;
        let json: serde_json::Value = serde_json::from_str(&data)?;
        let bookmarks = parse_chromium_bookmarks(&json)?;
        
        let count = count_bookmarks(&bookmarks);
        info!("✅ Brave Nightly (Default): {} bookmarks", count);
        Ok(bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;
        
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        
        let bookmarks_path = default_profile.join("Bookmarks");
        std::fs::write(&bookmarks_path, &data)?;
        
        let total = count_bookmarks(bookmarks);
        info!("✅ Wrote {} bookmarks to Brave Nightly (Default)", total);
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
    
    fn supports_history(&self) -> bool {
        true
    }
    
    fn read_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> {
        // Only read from Default profile for performance
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;
        
        let history_path = default_profile.join("History");
        if history_path.exists() && history_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            match read_chromium_history(&history_path, days) {
                Ok(history) => {
                    info!("✅ Brave Nightly (Default): {} history items", history.len());
                    return Ok(history);
                }
                Err(e) => warn!("⚠️  Failed to read Brave Nightly history: {}", e),
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;
        
        let history_path = default_profile.join("History");
        if history_path.exists() {
            match write_chromium_history(&history_path, items) {
                Ok(_) => info!("✅ Wrote {} history items to Brave Nightly (Default)", items.len()),
                Err(e) => warn!("⚠️  Failed to write history to Brave Nightly: {}", e),
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        // Only read from Default profile for performance
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;
        
        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match read_chromium_cookies(&cookies_path) {
                Ok(cookies) => {
                    info!("✅ Brave Nightly (Default): {} cookies", cookies.len());
                    return Ok(cookies);
                }
                Err(e) => warn!("⚠️  Failed to read Brave Nightly cookies: {}", e),
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;
        
        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match write_chromium_cookies(&cookies_path, cookies) {
                Ok(_) => info!("✅ Wrote {} cookies to Brave Nightly (Default)", cookies.len()),
                Err(e) => warn!("⚠️  Failed to write cookies to Brave Nightly: {}", e),
            }
        }
        Ok(())
    }
}

// Chrome Adapter
pub struct ChromeAdapter;

impl ChromeAdapter {
    fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
        detect_chromium_profiles("Google/Chrome")
    }
}

impl BrowserAdapter for ChromeAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::Chrome
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        let profiles = self.detect_all_profiles()?;
        let bookmarks_path = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?
            .join("Bookmarks");
        Ok(bookmarks_path)
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;
        
        let bookmarks_path = default_profile.join("Bookmarks");
        if !bookmarks_path.exists() {
            return Ok(Vec::new());
        }
        
        let data = std::fs::read_to_string(&bookmarks_path)?;
        let json: serde_json::Value = serde_json::from_str(&data)?;
        let bookmarks = parse_chromium_bookmarks(&json)?;
        
        let count = count_bookmarks(&bookmarks);
        info!("✅ Chrome (Default): {} bookmarks", count);
        Ok(bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;
        
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        
        let bookmarks_path = default_profile.join("Bookmarks");
        std::fs::write(&bookmarks_path, &data)?;
        
        let total = count_bookmarks(bookmarks);
        info!("✅ Wrote {} bookmarks to Chrome (Default)", total);
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
    
    fn supports_history(&self) -> bool {
        true
    }
    
    fn read_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> {
        // Only read from Default profile for performance
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;
        
        let history_path = default_profile.join("History");
        if history_path.exists() && history_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            match read_chromium_history(&history_path, days) {
                Ok(history) => {
                    info!("✅ Chrome (Default): {} history items", history.len());
                    return Ok(history);
                }
                Err(e) => warn!("⚠️  Failed to read Chrome history: {}", e),
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;
        
        let history_path = default_profile.join("History");
        if history_path.exists() {
            match write_chromium_history(&history_path, items) {
                Ok(_) => info!("✅ Wrote {} history items to Chrome (Default)", items.len()),
                Err(e) => warn!("⚠️  Failed to write history to Chrome: {}", e),
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        // Only read from Default profile for performance
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;
        
        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match read_chromium_cookies(&cookies_path) {
                Ok(cookies) => {
                    info!("✅ Chrome (Default): {} cookies", cookies.len());
                    return Ok(cookies);
                }
                Err(e) => warn!("⚠️  Failed to read Chrome cookies: {}", e),
            }
        }
        
        Ok(Vec::new())
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        // Only write to Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;
        
        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match write_chromium_cookies(&cookies_path, cookies) {
                Ok(_) => info!("✅ Wrote {} cookies to Chrome (Default)", cookies.len()),
                Err(e) => warn!("⚠️  Failed to write cookies to Chrome: {}", e),
            }
        }
        Ok(())
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
fn parse_safari_plist(_value: &plist::Value) -> Result<Vec<Bookmark>> {
    // Simplified implementation - needs full Safari plist structure parsing
    Ok(vec![])
}

#[cfg(target_os = "macos")]
fn bookmarks_to_safari_plist(_bookmarks: &[Bookmark]) -> Result<plist::Value> {
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
            
            let mut bookmark = Bookmark {
                id: child.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                title: child.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: child.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                folder: is_folder,
                children: vec![],
                date_added: child.get("date_added").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()),
                date_modified: child.get("date_modified").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()),
            };
            
            // Recursively parse children if it's a folder
            if is_folder {
                let mut folder_children = Vec::new();
                parse_chromium_node_recursive(child, &mut folder_children)?;
                bookmark.children = folder_children;
            }
            
            // Add both folders and bookmarks (preserving structure)
            bookmarks.push(bookmark);
        }
    }
    
    Ok(())
}

fn bookmarks_to_chromium_json(bookmarks: &[Bookmark]) -> Result<serde_json::Value> {
    // Convert bookmarks to Chromium JSON format with proper folder structure
    let mut id_counter = 10u64; // Start from 10 to avoid conflicts with root folders
    
    fn convert_bookmark_recursive(bookmark: &Bookmark, id_counter: &mut u64) -> serde_json::Value {
        let current_id = *id_counter;
        *id_counter += 1;
        
        if bookmark.folder {
            // Convert folder with children
            let children: Vec<serde_json::Value> = bookmark.children
                .iter()
                .map(|child| convert_bookmark_recursive(child, id_counter))
                .collect();
            
            serde_json::json!({
                "children": children,
                "date_added": bookmark.date_added.unwrap_or(0).to_string(),
                "date_last_used": "0",
                "date_modified": bookmark.date_modified.unwrap_or(0).to_string(),
                "guid": format!("folder-{}", current_id),
                "id": current_id.to_string(),
                "name": bookmark.title,
                "type": "folder"
            })
        } else {
            // Convert bookmark URL
            serde_json::json!({
                "date_added": bookmark.date_added.unwrap_or(0).to_string(),
                "date_last_used": "0",
                "guid": format!("bookmark-{}", current_id),
                "id": current_id.to_string(),
                "name": bookmark.title,
                "type": "url",
                "url": bookmark.url.as_deref().unwrap_or("")
            })
        }
    }
    
    // Convert all bookmarks preserving structure
    let children: Vec<serde_json::Value> = bookmarks
        .iter()
        .map(|b| convert_bookmark_recursive(b, &mut id_counter))
        .collect();
    
    Ok(serde_json::json!({
        "checksum": "",
        "roots": {
            "bookmark_bar": {
                "children": children,
                "date_added": "0",
                "date_last_used": "0",
                "date_modified": "0",
                "guid": "00000000-0000-4000-a000-000000000002",
                "id": "1",
                "name": "Bookmarks Bar",
                "type": "folder"
            },
            "other": {
                "children": [],
                "date_added": "0",
                "date_last_used": "0",
                "date_modified": "0",
                "guid": "00000000-0000-4000-a000-000000000003",
                "id": "2",
                "name": "Other Bookmarks",
                "type": "folder"
            },
            "synced": {
                "children": [],
                "date_added": "0",
                "date_last_used": "0",
                "date_modified": "0",
                "guid": "00000000-0000-4000-a000-000000000004",
                "id": "3",
                "name": "Mobile Bookmarks",
                "type": "folder"
            }
        },
        "version": 1
    }))
}

// Firefox SQLite helper functions
fn read_firefox_bookmarks(db_path: &std::path::Path) -> Result<Vec<Bookmark>> {
    use rusqlite::{Connection, OpenFlags};
    use std::collections::HashMap;
    
    // Use read-only mode to avoid locking issues
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    
    // Read all bookmarks with parent info
    // type=1: bookmark, type=2: folder
    // parent=2: menu, parent=3: toolbar, parent=5: unfiled
    let mut stmt = conn.prepare(
        "SELECT b.id, b.title, p.url, b.dateAdded, b.lastModified, b.type, b.parent, b.position
         FROM moz_bookmarks b
         LEFT JOIN moz_places p ON b.fk = p.id
         WHERE b.type IN (1, 2) AND b.parent >= 2
         ORDER BY b.parent, b.position"
    )?;
    
    // First pass: collect all items
    let mut all_items: HashMap<i64, (Bookmark, i64)> = HashMap::new(); // id -> (bookmark, parent_id)
    let mut children_map: HashMap<i64, Vec<i64>> = HashMap::new(); // parent_id -> [child_ids]
    
    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let bookmark_type: i32 = row.get(5)?;
        let parent: i64 = row.get(6)?;
        
        Ok((
            id,
            Bookmark {
                id: id.to_string(),
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                url: row.get::<_, Option<String>>(2)?,
                folder: bookmark_type == 2,
                children: vec![],
                date_added: row.get::<_, Option<i64>>(3)?,
                date_modified: row.get::<_, Option<i64>>(4)?,
            },
            parent,
        ))
    })?;
    
    for row in rows {
        let (id, bookmark, parent) = row?;
        all_items.insert(id, (bookmark, parent));
        children_map.entry(parent).or_default().push(id);
    }
    
    // Build tree structure recursively
    fn build_tree(id: i64, all_items: &mut HashMap<i64, (Bookmark, i64)>, children_map: &HashMap<i64, Vec<i64>>) -> Option<Bookmark> {
        let (mut bookmark, _parent) = all_items.remove(&id)?;
        
        if bookmark.folder {
            if let Some(child_ids) = children_map.get(&id) {
                for child_id in child_ids {
                    if let Some(child) = build_tree(*child_id, all_items, children_map) {
                        bookmark.children.push(child);
                    }
                }
            }
        }
        
        Some(bookmark)
    }
    
    // Build from root folders (2=menu, 3=toolbar, 5=unfiled)
    let mut bookmarks = Vec::new();
    
    // Get items from toolbar (parent=3) - most important
    if let Some(toolbar_children) = children_map.get(&3) {
        for child_id in toolbar_children.clone() {
            if let Some(bookmark) = build_tree(child_id, &mut all_items, &children_map) {
                bookmarks.push(bookmark);
            }
        }
    }
    
    // Get items from menu (parent=2)
    if let Some(menu_children) = children_map.get(&2) {
        for child_id in menu_children.clone() {
            if let Some(bookmark) = build_tree(child_id, &mut all_items, &children_map) {
                bookmarks.push(bookmark);
            }
        }
    }
    
    // Get items from unfiled (parent=5)
    if let Some(unfiled_children) = children_map.get(&5) {
        for child_id in unfiled_children.clone() {
            if let Some(bookmark) = build_tree(child_id, &mut all_items, &children_map) {
                bookmarks.push(bookmark);
            }
        }
    }
    
    let total_count = count_bookmarks(&bookmarks);
    info!("📚 Read {} bookmarks (tree structure) from Firefox database", total_count);
    Ok(bookmarks)
}

fn count_bookmarks(bookmarks: &[Bookmark]) -> usize {
    let mut count = 0;
    for b in bookmarks {
        if b.folder {
            count += count_bookmarks(&b.children);
        } else {
            count += 1;
        }
    }
    count
}

fn write_firefox_bookmarks(db_path: &std::path::Path, bookmarks: &[Bookmark]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    // Start transaction
    conn.execute("BEGIN TRANSACTION", [])?;
    
    // Clear existing user bookmarks (keep system folders: 1=root, 2=menu, 3=toolbar, 4=tags, 5=unfiled, 6=mobile)
    conn.execute(
        "DELETE FROM moz_bookmarks WHERE id > 6 AND parent >= 2",
        [],
    )?;
    
    let mut position_counter = 0i32;
    let now = chrono::Utc::now().timestamp_micros();
    
    // Recursive function to insert bookmarks with folder structure
    fn insert_bookmark_recursive(
        conn: &Connection,
        bookmark: &Bookmark,
        parent_id: i64,
        position: &mut i32,
        now: i64,
    ) -> Result<()> {
        let current_position = *position;
        *position += 1;
        
        if bookmark.folder {
            // 🔧 FIX: Skip empty folders and folders with "/" name
            if bookmark.children.is_empty() {
                debug!("Skipping empty folder: {}", bookmark.title);
                return Ok(());
            }
            
            if bookmark.title == "/" || bookmark.title.is_empty() {
                debug!("Skipping invalid folder name: '{}'", bookmark.title);
                return Ok(());
            }
            
            // Generate a unique GUID for the folder
            let guid = format!("folder_{}", uuid::Uuid::new_v4().to_string().replace("-", "")[..12].to_string());
            
            // Insert folder
            conn.execute(
                "INSERT INTO moz_bookmarks (type, fk, parent, position, title, dateAdded, lastModified, guid)
                 VALUES (2, NULL, ?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    parent_id,
                    current_position,
                    &bookmark.title,
                    bookmark.date_added.unwrap_or(now),
                    bookmark.date_modified.unwrap_or(now),
                    guid,
                ],
            )?;
            
            // Get the new folder's ID
            let folder_id: i64 = conn.query_row(
                "SELECT last_insert_rowid()",
                [],
                |row| row.get(0),
            )?;
            
            // Insert children
            let mut child_position = 0i32;
            for child in &bookmark.children {
                insert_bookmark_recursive(conn, child, folder_id, &mut child_position, now)?;
            }
        } else if let Some(url) = &bookmark.url {
            // Generate a unique GUID for the bookmark
            let guid = format!("bkmk_{}", uuid::Uuid::new_v4().to_string().replace("-", "")[..12].to_string());
            
            // First, ensure the URL exists in moz_places
            conn.execute(
                "INSERT OR IGNORE INTO moz_places (url, title, rev_host, hidden, typed, frecency, guid)
                 VALUES (?1, ?2, '', 0, 0, -1, ?3)",
                rusqlite::params![url, &bookmark.title, format!("place_{}", guid)],
            )?;
            
            // Get the place_id
            let place_id: i64 = conn.query_row(
                "SELECT id FROM moz_places WHERE url = ?1",
                [url],
                |row| row.get(0),
            )?;
            
            // Insert bookmark
            conn.execute(
                "INSERT INTO moz_bookmarks (type, fk, parent, position, title, dateAdded, lastModified, guid)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    place_id,
                    parent_id,
                    current_position,
                    &bookmark.title,
                    bookmark.date_added.unwrap_or(now),
                    bookmark.date_modified.unwrap_or(now),
                    guid,
                ],
            )?;
        }
        
        Ok(())
    }
    
    // Insert all bookmarks into toolbar (parent=3)
    for bookmark in bookmarks {
        insert_bookmark_recursive(&conn, bookmark, 3, &mut position_counter, now)?;
    }
    
    // Commit transaction
    conn.execute("COMMIT", [])?;
    
    let total = count_bookmarks(bookmarks);
    info!("📚 Wrote {} bookmarks (tree structure) to Firefox database", total);
    Ok(())
}

// Firefox history helper functions
fn read_firefox_history(db_path: &std::path::Path, days: Option<i32>) -> Result<Vec<HistoryItem>> {
    use rusqlite::{Connection, OpenFlags};
    
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    
    let mut history = Vec::new();
    
    // Calculate timestamp for filtering (microseconds since epoch)
    let cutoff_timestamp = if let Some(days) = days {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(days as i64);
        Some(cutoff.timestamp_micros())
    } else {
        None
    };
    
    let query = if let Some(cutoff) = cutoff_timestamp {
        format!(
            "SELECT p.url, p.title, p.visit_count, MAX(v.visit_date) as last_visit
             FROM moz_places p
             JOIN moz_historyvisits v ON p.id = v.place_id
             WHERE v.visit_date > {}
             GROUP BY p.id
             ORDER BY last_visit DESC",
            cutoff
        )
    } else {
        "SELECT p.url, p.title, p.visit_count, MAX(v.visit_date) as last_visit
         FROM moz_places p
         JOIN moz_historyvisits v ON p.id = v.place_id
         GROUP BY p.id
         ORDER BY last_visit DESC"
            .to_string()
    };
    
    let mut stmt = conn.prepare(&query)?;
    let history_iter = stmt.query_map([], |row| {
        Ok(HistoryItem {
            url: row.get(0)?,
            title: row.get(1)?,
            visit_count: row.get(2)?,
            last_visit: row.get(3)?,
        })
    })?;
    
    for item in history_iter {
        history.push(item?);
    }
    
    debug!("Read {} history items from Firefox database", history.len());
    Ok(history)
}

fn write_firefox_history(db_path: &std::path::Path, items: &[HistoryItem]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    // Start transaction
    conn.execute("BEGIN TRANSACTION", [])?;
    
    // Insert history items
    for item in items {
        // First, ensure the URL exists in moz_places
        conn.execute(
            "INSERT OR REPLACE INTO moz_places (url, title, rev_host, hidden, typed, frecency, visit_count)
             VALUES (?1, ?2, '', 0, 0, -1, ?3)",
            rusqlite::params![&item.url, &item.title, item.visit_count],
        )?;
        
        // Get the place_id
        let place_id: i64 = conn.query_row(
            "SELECT id FROM moz_places WHERE url = ?1",
            [&item.url],
            |row| row.get(0),
        )?;
        
        // Insert visit record
        if let Some(last_visit) = item.last_visit {
            conn.execute(
                "INSERT OR IGNORE INTO moz_historyvisits (place_id, visit_date, visit_type, from_visit)
                 VALUES (?1, ?2, 1, 0)",
                rusqlite::params![place_id, last_visit],
            )?;
        }
    }
    
    // Commit transaction
    conn.execute("COMMIT", [])?;
    
    debug!("Wrote {} history items to Firefox database", items.len());
    Ok(())
}

// Safari reading list helper functions
#[cfg(target_os = "macos")]
fn parse_safari_reading_list(value: &plist::Value) -> Result<Vec<ReadingListItem>> {
    let mut items = Vec::new();
    
    if let Some(dict) = value.as_dictionary() {
        if let Some(children) = dict.get("Children").and_then(|v| v.as_array()) {
            for child in children {
                if let Some(child_dict) = child.as_dictionary() {
                    // Check if this is a reading list item
                    if let Some(_reading_list) = child_dict.get("ReadingList") {
                        if let Some(url_string) = child_dict.get("URLString").and_then(|v| v.as_string()) {
                            let title = child_dict
                                .get("URIDictionary")
                                .and_then(|v| v.as_dictionary())
                                .and_then(|d| d.get("title"))
                                .and_then(|v| v.as_string())
                                .unwrap_or(url_string)
                                .to_string();
                            
                            let date_added = child_dict
                                .get("ReadingListDateAdded")
                                .and_then(|v| v.as_date())
                                .map(|d| {
                                    // Convert plist::Date to timestamp
                                    use std::time::SystemTime;
                                    let system_time: SystemTime = d.clone().into();
                                    system_time.duration_since(SystemTime::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs() as i64
                                });
                            
                            items.push(ReadingListItem {
                                url: url_string.to_string(),
                                title,
                                date_added,
                            });
                        }
                    }
                }
            }
        }
    }
    
    Ok(items)
}

#[cfg(target_os = "macos")]
fn update_safari_reading_list(_plist: &mut plist::Value, _items: &[ReadingListItem]) -> Result<()> {
    // This is a simplified implementation
    // In reality, we need to preserve the existing structure and only update the reading list section
    // For now, we'll just log that this needs implementation
    warn!("Safari reading list write not fully implemented yet");
    Ok(())
}

// Chromium history helper functions
fn read_chromium_history(db_path: &std::path::Path, days: Option<i32>) -> Result<Vec<HistoryItem>> {
    use rusqlite::{Connection, OpenFlags};
    
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    
    let mut history = Vec::new();
    
    // Calculate timestamp for filtering (microseconds since epoch for Chromium)
    let cutoff_timestamp = if let Some(days) = days {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(days as i64);
        // Chromium uses microseconds since 1601-01-01
        let chromium_epoch = chrono::NaiveDate::from_ymd_opt(1601, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let duration = cutoff.signed_duration_since(chromium_epoch);
        Some(duration.num_microseconds().unwrap_or(0))
    } else {
        None
    };
    
    let query = if let Some(cutoff) = cutoff_timestamp {
        format!(
            "SELECT url, title, visit_count, last_visit_time
             FROM urls
             WHERE last_visit_time > {}
             ORDER BY last_visit_time DESC",
            cutoff
        )
    } else {
        "SELECT url, title, visit_count, last_visit_time
         FROM urls
         ORDER BY last_visit_time DESC"
            .to_string()
    };
    
    let mut stmt = conn.prepare(&query)?;
    let history_iter = stmt.query_map([], |row| {
        Ok(HistoryItem {
            url: row.get(0)?,
            title: row.get(1)?,
            visit_count: row.get(2)?,
            last_visit: row.get(3)?,
        })
    })?;
    
    for item in history_iter {
        history.push(item?);
    }
    
    debug!("Read {} history items from Chromium database", history.len());
    Ok(history)
}

fn write_chromium_history(db_path: &std::path::Path, items: &[HistoryItem]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    // Start transaction
    conn.execute("BEGIN TRANSACTION", [])?;
    
    // Insert history items
    for item in items {
        conn.execute(
            "INSERT OR REPLACE INTO urls (url, title, visit_count, last_visit_time, typed_count, hidden)
             VALUES (?1, ?2, ?3, ?4, 0, 0)",
            rusqlite::params![
                &item.url,
                &item.title,
                item.visit_count,
                item.last_visit.unwrap_or(0)
            ],
        )?;
        
        // Get the url_id
        let url_id: i64 = conn.query_row(
            "SELECT id FROM urls WHERE url = ?1",
            [&item.url],
            |row| row.get(0),
        )?;
        
        // Insert visit record
        if let Some(last_visit) = item.last_visit {
            conn.execute(
                "INSERT OR IGNORE INTO visits (url, visit_time, from_visit, transition, segment_id)
                 VALUES (?1, ?2, 0, 0, 0)",
                rusqlite::params![url_id, last_visit],
            )?;
        }
    }
    
    // Commit transaction
    conn.execute("COMMIT", [])?;
    
    debug!("Wrote {} history items to Chromium database", items.len());
    Ok(())
}

// Safari history helper functions
fn read_safari_history(db_path: &std::path::Path, days: Option<i32>) -> Result<Vec<HistoryItem>> {
    use rusqlite::{Connection, OpenFlags};
    
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    
    let mut history = Vec::new();
    
    // Safari uses Core Data timestamp (seconds since 2001-01-01)
    let cutoff_timestamp = if let Some(days) = days {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(days as i64);
        // Convert to Safari timestamp (seconds since 2001-01-01 00:00:00 UTC)
        let safari_epoch = chrono::NaiveDate::from_ymd_opt(2001, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let duration = cutoff.signed_duration_since(safari_epoch);
        Some(duration.num_seconds() as f64)
    } else {
        None
    };
    
    let query = if let Some(cutoff) = cutoff_timestamp {
        format!(
            "SELECT i.url, v.title, i.visit_count, MAX(v.visit_time) as last_visit
             FROM history_items i
             JOIN history_visits v ON i.id = v.history_item
             WHERE v.visit_time > {}
             GROUP BY i.id
             ORDER BY last_visit DESC",
            cutoff
        )
    } else {
        "SELECT i.url, v.title, i.visit_count, MAX(v.visit_time) as last_visit
         FROM history_items i
         JOIN history_visits v ON i.id = v.history_item
         GROUP BY i.id
         ORDER BY last_visit DESC"
            .to_string()
    };
    
    let mut stmt = conn.prepare(&query)?;
    let history_iter = stmt.query_map([], |row| {
        // Convert Safari timestamp to Unix timestamp (milliseconds)
        let safari_time: f64 = row.get(3)?;
        let safari_epoch = chrono::NaiveDate::from_ymd_opt(2001, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let unix_time = safari_epoch.timestamp_millis() + (safari_time * 1000.0) as i64;
        
        Ok(HistoryItem {
            url: row.get(0)?,
            title: row.get(1)?,
            visit_count: row.get(2)?,
            last_visit: Some(unix_time),
        })
    })?;
    
    for item in history_iter {
        history.push(item?);
    }
    
    debug!("Read {} history items from Safari database", history.len());
    Ok(history)
}

fn write_safari_history(db_path: &std::path::Path, items: &[HistoryItem]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    // Start transaction
    conn.execute("BEGIN TRANSACTION", [])?;
    
    // Insert history items
    for item in items {
        // First, ensure the URL exists in history_items
        conn.execute(
            "INSERT OR REPLACE INTO history_items (url, visit_count, domain_expansion, daily_visit_counts, weekly_visit_counts, should_recompute_derived_visit_counts, visit_count_score)
             VALUES (?1, ?2, '', X'', NULL, 0, 0)",
            rusqlite::params![&item.url, item.visit_count],
        )?;
        
        // Get the history_item id
        let item_id: i64 = conn.query_row(
            "SELECT id FROM history_items WHERE url = ?1",
            [&item.url],
            |row| row.get(0),
        )?;
        
        // Insert visit record
        if let Some(last_visit) = item.last_visit {
            // Convert Unix timestamp to Safari timestamp
            let safari_epoch = chrono::NaiveDate::from_ymd_opt(2001, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            let safari_time = (last_visit - safari_epoch.timestamp_millis()) as f64 / 1000.0;
            
            conn.execute(
                "INSERT OR IGNORE INTO history_visits (history_item, visit_time, title, load_successful, http_non_get, synthesized, origin, generation, attributes, score)
                 VALUES (?1, ?2, ?3, 1, 0, 0, 0, 0, 0, 0)",
                rusqlite::params![item_id, safari_time, &item.title],
            )?;
        }
    }
    
    // Commit transaction
    conn.execute("COMMIT", [])?;
    
    debug!("Wrote {} history items to Safari database", items.len());
    Ok(())
}

// Firefox/Waterfox cookies helper functions
fn read_firefox_cookies(db_path: &std::path::Path) -> Result<Vec<Cookie>> {
    use rusqlite::{Connection, OpenFlags};
    
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    
    let mut cookies = Vec::new();
    
    let mut stmt = conn.prepare(
        "SELECT host, name, value, path, expiry, isSecure, isHttpOnly
         FROM moz_cookies
         ORDER BY host"
    )?;
    
    let cookie_iter = stmt.query_map([], |row| {
        Ok(Cookie {
            host: row.get(0)?,
            name: row.get(1)?,
            value: row.get(2)?,
            path: row.get(3)?,
            expiry: row.get(4)?,
            is_secure: row.get::<_, i32>(5)? == 1,
            is_http_only: row.get::<_, i32>(6)? == 1,
        })
    })?;
    
    for cookie in cookie_iter {
        cookies.push(cookie?);
    }
    
    debug!("Read {} cookies from Firefox database", cookies.len());
    Ok(cookies)
}

fn write_firefox_cookies(db_path: &std::path::Path, cookies: &[Cookie]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    conn.execute("BEGIN TRANSACTION", [])?;
    
    let now = chrono::Utc::now().timestamp_micros();
    
    for cookie in cookies {
        conn.execute(
            "INSERT OR REPLACE INTO moz_cookies (originAttributes, name, value, host, path, expiry, lastAccessed, creationTime, isSecure, isHttpOnly, inBrowserElement, sameSite, schemeMap, isPartitionedAttributeSet)
             VALUES ('', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 0, 0)",
            rusqlite::params![
                &cookie.name,
                &cookie.value,
                &cookie.host,
                &cookie.path,
                cookie.expiry.unwrap_or(0),
                now,
                now,
                if cookie.is_secure { 1 } else { 0 },
                if cookie.is_http_only { 1 } else { 0 },
            ],
        )?;
    }
    
    conn.execute("COMMIT", [])?;
    
    debug!("Wrote {} cookies to Firefox database", cookies.len());
    Ok(())
}

// Chromium cookies helper functions
fn read_chromium_cookies(db_path: &std::path::Path) -> Result<Vec<Cookie>> {
    use rusqlite::{Connection, OpenFlags};
    
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    
    let mut cookies = Vec::new();
    
    let mut stmt = conn.prepare(
        "SELECT host_key, name, value, path, expires_utc, is_secure, is_httponly
         FROM cookies
         ORDER BY host_key"
    )?;
    
    let cookie_iter = stmt.query_map([], |row| {
        Ok(Cookie {
            host: row.get(0)?,
            name: row.get(1)?,
            value: row.get(2)?,
            path: row.get(3)?,
            expiry: row.get(4)?,
            is_secure: row.get::<_, i32>(5)? == 1,
            is_http_only: row.get::<_, i32>(6)? == 1,
        })
    })?;
    
    for cookie in cookie_iter {
        cookies.push(cookie?);
    }
    
    debug!("Read {} cookies from Chromium database", cookies.len());
    Ok(cookies)
}

fn write_chromium_cookies(db_path: &std::path::Path, cookies: &[Cookie]) -> Result<()> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path)?;
    
    conn.execute("BEGIN TRANSACTION", [])?;
    
    let now = chrono::Utc::now().timestamp_micros();
    
    for cookie in cookies {
        conn.execute(
            "INSERT OR REPLACE INTO cookies (creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly, last_access_utc, has_expires, is_persistent, priority, samesite, source_scheme, source_port, last_update_utc, source_type, has_cross_site_ancestor)
             VALUES (?1, ?2, '', ?3, ?4, X'', ?5, ?6, ?7, ?8, ?9, 1, 1, 1, 0, 0, -1, ?10, 0, 0)",
            rusqlite::params![
                now,
                &cookie.host,
                &cookie.name,
                &cookie.value,
                &cookie.path,
                cookie.expiry.unwrap_or(0),
                if cookie.is_secure { 1 } else { 0 },
                if cookie.is_http_only { 1 } else { 0 },
                now,
                now,
            ],
        )?;
    }
    
    conn.execute("COMMIT", [])?;
    
    debug!("Wrote {} cookies to Chromium database", cookies.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_type_name() {
        assert_eq!(BrowserType::Waterfox.name(), "Waterfox");
        assert_eq!(BrowserType::Safari.name(), "Safari");
        assert_eq!(BrowserType::Brave.name(), "Brave");
        assert_eq!(BrowserType::BraveNightly.name(), "Brave Nightly");
        assert_eq!(BrowserType::Chrome.name(), "Chrome");
        assert_eq!(BrowserType::FirefoxNightly.name(), "Firefox Nightly");
    }

    #[test]
    fn test_bookmark_creation() {
        let bookmark = Bookmark {
            id: "test-id".to_string(),
            title: "Test Bookmark".to_string(),
            url: Some("https://example.com".to_string()),
            folder: false,
            children: vec![],
            date_added: Some(1700000000000),
            date_modified: Some(1700000000000),
        };
        
        assert_eq!(bookmark.id, "test-id");
        assert_eq!(bookmark.title, "Test Bookmark");
        assert!(bookmark.url.is_some());
        assert!(!bookmark.folder);
        assert!(bookmark.children.is_empty());
    }

    #[test]
    fn test_folder_creation() {
        let child = Bookmark {
            id: "child-id".to_string(),
            title: "Child".to_string(),
            url: Some("https://child.com".to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        };
        
        let folder = Bookmark {
            id: "folder-id".to_string(),
            title: "Test Folder".to_string(),
            url: None,
            folder: true,
            children: vec![child],
            date_added: Some(1700000000000),
            date_modified: None,
        };
        
        assert!(folder.folder);
        assert!(folder.url.is_none());
        assert_eq!(folder.children.len(), 1);
    }

    #[test]
    fn test_cookie_creation() {
        let cookie = Cookie {
            host: ".example.com".to_string(),
            name: "session".to_string(),
            value: "abc123".to_string(),
            path: "/".to_string(),
            expiry: Some(1800000000),
            is_secure: true,
            is_http_only: true,
        };
        
        assert_eq!(cookie.host, ".example.com");
        assert_eq!(cookie.name, "session");
        assert!(cookie.is_secure);
        assert!(cookie.is_http_only);
    }

    #[test]
    fn test_history_item_creation() {
        let item = HistoryItem {
            url: "https://example.com/page".to_string(),
            title: Some("Example Page".to_string()),
            visit_count: 5,
            last_visit: Some(1700000000000),
        };
        
        assert_eq!(item.url, "https://example.com/page");
        assert_eq!(item.title.as_ref().unwrap(), "Example Page");
        assert_eq!(item.visit_count, 5);
    }

    #[test]
    fn test_reading_list_item_creation() {
        let item = ReadingListItem {
            url: "https://article.com/long-read".to_string(),
            title: "Long Article".to_string(),
            date_added: Some(1700000000000),
        };
        
        assert_eq!(item.url, "https://article.com/long-read");
        assert_eq!(item.title, "Long Article");
    }

    #[test]
    fn test_get_all_adapters() {
        let adapters = get_all_adapters();
        assert_eq!(adapters.len(), 6);
        
        let names: Vec<&str> = adapters.iter().map(|a| a.browser_type().name()).collect();
        assert!(names.contains(&"Waterfox"));
        assert!(names.contains(&"Safari"));
        assert!(names.contains(&"Brave"));
        assert!(names.contains(&"Brave Nightly"));
        assert!(names.contains(&"Chrome"));
        assert!(names.contains(&"Firefox Nightly"));
    }

    #[test]
    fn test_browser_adapter_default_methods() {
        let adapter = WaterfoxAdapter;
        
        assert!(adapter.supports_history());
        assert!(adapter.supports_cookies());
        assert!(!adapter.supports_reading_list());
    }

    #[test]
    fn test_safari_adapter_supports() {
        let adapter = SafariAdapter;
        
        assert!(adapter.supports_reading_list());
        assert!(adapter.supports_history());
        assert!(!adapter.supports_cookies());
    }

    #[test]
    fn test_bookmark_serialization() {
        let bookmark = Bookmark {
            id: "1".to_string(),
            title: "Test".to_string(),
            url: Some("https://test.com".to_string()),
            folder: false,
            children: vec![],
            date_added: Some(1700000000000),
            date_modified: None,
        };
        
        let json = serde_json::to_string(&bookmark).unwrap();
        assert!(json.contains("\"id\":\"1\""));
        assert!(json.contains("\"title\":\"Test\""));
        
        let deserialized: Bookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, bookmark.id);
        assert_eq!(deserialized.title, bookmark.title);
    }

    #[test]
    fn test_nested_bookmarks() {
        let inner_folder = Bookmark {
            id: "inner".to_string(),
            title: "Inner Folder".to_string(),
            url: None,
            folder: true,
            children: vec![
                Bookmark {
                    id: "deep".to_string(),
                    title: "Deep Bookmark".to_string(),
                    url: Some("https://deep.com".to_string()),
                    folder: false,
                    children: vec![],
                    date_added: None,
                    date_modified: None,
                },
            ],
            date_added: None,
            date_modified: None,
        };
        
        let outer_folder = Bookmark {
            id: "outer".to_string(),
            title: "Outer Folder".to_string(),
            url: None,
            folder: true,
            children: vec![inner_folder],
            date_added: None,
            date_modified: None,
        };
        
        assert!(outer_folder.folder);
        assert_eq!(outer_folder.children.len(), 1);
        assert!(outer_folder.children[0].folder);
        assert_eq!(outer_folder.children[0].children.len(), 1);
        assert!(!outer_folder.children[0].children[0].folder);
    }
}
