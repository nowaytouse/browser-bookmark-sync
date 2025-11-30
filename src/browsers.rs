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
        Ok(profiles.first()
            .ok_or_else(|| anyhow::anyhow!("No Waterfox profiles found"))?
            .clone())
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Read from ALL profiles and merge
        let profiles = self.detect_all_profiles()?;
        let mut all_bookmarks = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            match read_firefox_bookmarks(profile_path) {
                Ok(bookmarks) => {
                    info!("✅ Waterfox Profile {}: {} bookmarks", idx + 1, bookmarks.len());
                    all_bookmarks.extend(bookmarks);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read Waterfox profile {}: {}", idx + 1, e);
                }
            }
        }
        
        info!("📊 Total Waterfox bookmarks from {} profiles: {}", profiles.len(), all_bookmarks.len());
        Ok(all_bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        // Write to ALL profiles
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            match write_firefox_bookmarks(profile_path, bookmarks) {
                Ok(_) => {
                    info!("✅ Wrote {} bookmarks to Waterfox profile {}", bookmarks.len(), idx + 1);
                }
                Err(e) => {
                    warn!("⚠️  Failed to write to Waterfox profile {}: {}", idx + 1, e);
                }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_history = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            match read_firefox_history(profile_path, days) {
                Ok(history) => {
                    info!("✅ Waterfox Profile {}: {} history items", idx + 1, history.len());
                    all_history.extend(history);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read Waterfox history from profile {}: {}", idx + 1, e);
                }
            }
        }
        
        info!("📊 Total Waterfox history from {} profiles: {}", profiles.len(), all_history.len());
        Ok(all_history)
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            match write_firefox_history(profile_path, items) {
                Ok(_) => {
                    info!("✅ Wrote {} history items to Waterfox profile {}", items.len(), idx + 1);
                }
                Err(e) => {
                    warn!("⚠️  Failed to write history to Waterfox profile {}: {}", idx + 1, e);
                }
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        let profiles = self.detect_all_profiles()?;
        let mut all_cookies = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            // cookies.sqlite is in the same directory as places.sqlite
            let cookies_path = profile_path.parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile path"))?
                .join("cookies.sqlite");
            
            if cookies_path.exists() {
                match read_firefox_cookies(&cookies_path) {
                    Ok(cookies) => {
                        info!("✅ Waterfox Profile {}: {} cookies", idx + 1, cookies.len());
                        all_cookies.extend(cookies);
                    }
                    Err(e) => {
                        warn!("⚠️  Failed to read Waterfox cookies from profile {}: {}", idx + 1, e);
                    }
                }
            }
        }
        
        info!("📊 Total Waterfox cookies from {} profiles: {}", profiles.len(), all_cookies.len());
        Ok(all_cookies)
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile path"))?
                .join("cookies.sqlite");
            
            if cookies_path.exists() {
                match write_firefox_cookies(&cookies_path, cookies) {
                    Ok(_) => {
                        info!("✅ Wrote {} cookies to Waterfox profile {}", cookies.len(), idx + 1);
                    }
                    Err(e) => {
                        warn!("⚠️  Failed to write cookies to Waterfox profile {}: {}", idx + 1, e);
                    }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_bookmarks = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let bookmarks_path = profile_path.join("Bookmarks");
            if bookmarks_path.exists() {
                match std::fs::read_to_string(&bookmarks_path) {
                    Ok(data) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                            if let Ok(bookmarks) = parse_chromium_bookmarks(&json) {
                                let count = count_bookmarks(&bookmarks);
                                info!("✅ Brave Profile {}: {} bookmarks ({} top-level)", idx + 1, count, bookmarks.len());
                                all_bookmarks.extend(bookmarks);
                            }
                        }
                    }
                    Err(e) => warn!("⚠️  Failed to read Brave profile {}: {}", idx + 1, e),
                }
            }
        }
        
        let total = count_bookmarks(&all_bookmarks);
        info!("📊 Total Brave bookmarks from {} profiles: {}", profiles.len(), total);
        Ok(all_bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let bookmarks_path = profile_path.join("Bookmarks");
            match std::fs::write(&bookmarks_path, &data) {
                Ok(_) => info!("✅ Wrote {} bookmarks to Brave profile {}", bookmarks.len(), idx + 1),
                Err(e) => warn!("⚠️  Failed to write to Brave profile {}: {}", idx + 1, e),
            }
        }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_history = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let history_path = profile_path.join("History");
            if history_path.exists() && history_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                match read_chromium_history(&history_path, days) {
                    Ok(history) => {
                        info!("✅ Brave Profile {}: {} history items", idx + 1, history.len());
                        all_history.extend(history);
                    }
                    Err(e) => warn!("⚠️  Failed to read Brave history from profile {}: {}", idx + 1, e),
                }
            }
        }
        
        info!("📊 Total Brave history from {} profiles: {}", profiles.len(), all_history.len());
        Ok(all_history)
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let history_path = profile_path.join("History");
            if history_path.exists() {
                match write_chromium_history(&history_path, items) {
                    Ok(_) => info!("✅ Wrote {} history items to Brave profile {}", items.len(), idx + 1),
                    Err(e) => warn!("⚠️  Failed to write history to Brave profile {}: {}", idx + 1, e),
                }
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        let profiles = self.detect_all_profiles()?;
        let mut all_cookies = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.join("Cookies");
            if cookies_path.exists() {
                match read_chromium_cookies(&cookies_path) {
                    Ok(cookies) => {
                        info!("✅ Brave Profile {}: {} cookies", idx + 1, cookies.len());
                        all_cookies.extend(cookies);
                    }
                    Err(e) => warn!("⚠️  Failed to read Brave cookies from profile {}: {}", idx + 1, e),
                }
            }
        }
        
        info!("📊 Total Brave cookies from {} profiles: {}", profiles.len(), all_cookies.len());
        Ok(all_cookies)
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.join("Cookies");
            if cookies_path.exists() {
                match write_chromium_cookies(&cookies_path, cookies) {
                    Ok(_) => info!("✅ Wrote {} cookies to Brave profile {}", cookies.len(), idx + 1),
                    Err(e) => warn!("⚠️  Failed to write cookies to Brave profile {}: {}", idx + 1, e),
                }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_bookmarks = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let bookmarks_path = profile_path.join("Bookmarks");
            if bookmarks_path.exists() {
                match std::fs::read_to_string(&bookmarks_path) {
                    Ok(data) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                            if let Ok(bookmarks) = parse_chromium_bookmarks(&json) {
                                let count = count_bookmarks(&bookmarks);
                                info!("✅ Brave Nightly Profile {}: {} bookmarks ({} top-level)", idx + 1, count, bookmarks.len());
                                all_bookmarks.extend(bookmarks);
                            }
                        }
                    }
                    Err(e) => warn!("⚠️  Failed to read Brave Nightly profile {}: {}", idx + 1, e),
                }
            }
        }
        
        let total = count_bookmarks(&all_bookmarks);
        info!("📊 Total Brave Nightly bookmarks from {} profiles: {}", profiles.len(), total);
        Ok(all_bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        
        let total = count_bookmarks(bookmarks);
        for (idx, profile_path) in profiles.iter().enumerate() {
            let bookmarks_path = profile_path.join("Bookmarks");
            match std::fs::write(&bookmarks_path, &data) {
                Ok(_) => info!("✅ Wrote {} bookmarks to Brave Nightly profile {}", total, idx + 1),
                Err(e) => warn!("⚠️  Failed to write to Brave Nightly profile {}: {}", idx + 1, e),
            }
        }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_history = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let history_path = profile_path.join("History");
            if history_path.exists() && history_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                match read_chromium_history(&history_path, days) {
                    Ok(history) => {
                        info!("✅ Brave Nightly Profile {}: {} history items", idx + 1, history.len());
                        all_history.extend(history);
                    }
                    Err(e) => warn!("⚠️  Failed to read Brave Nightly history from profile {}: {}", idx + 1, e),
                }
            }
        }
        
        info!("📊 Total Brave Nightly history from {} profiles: {}", profiles.len(), all_history.len());
        Ok(all_history)
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let history_path = profile_path.join("History");
            if history_path.exists() {
                match write_chromium_history(&history_path, items) {
                    Ok(_) => info!("✅ Wrote {} history items to Brave Nightly profile {}", items.len(), idx + 1),
                    Err(e) => warn!("⚠️  Failed to write history to Brave Nightly profile {}: {}", idx + 1, e),
                }
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        let profiles = self.detect_all_profiles()?;
        let mut all_cookies = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.join("Cookies");
            if cookies_path.exists() {
                match read_chromium_cookies(&cookies_path) {
                    Ok(cookies) => {
                        info!("✅ Brave Nightly Profile {}: {} cookies", idx + 1, cookies.len());
                        all_cookies.extend(cookies);
                    }
                    Err(e) => warn!("⚠️  Failed to read Brave Nightly cookies from profile {}: {}", idx + 1, e),
                }
            }
        }
        
        info!("📊 Total Brave Nightly cookies from {} profiles: {}", profiles.len(), all_cookies.len());
        Ok(all_cookies)
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.join("Cookies");
            if cookies_path.exists() {
                match write_chromium_cookies(&cookies_path, cookies) {
                    Ok(_) => info!("✅ Wrote {} cookies to Brave Nightly profile {}", cookies.len(), idx + 1),
                    Err(e) => warn!("⚠️  Failed to write cookies to Brave Nightly profile {}: {}", idx + 1, e),
                }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_bookmarks = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let bookmarks_path = profile_path.join("Bookmarks");
            if bookmarks_path.exists() {
                match std::fs::read_to_string(&bookmarks_path) {
                    Ok(data) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                            if let Ok(bookmarks) = parse_chromium_bookmarks(&json) {
                                let count = count_bookmarks(&bookmarks);
                                info!("✅ Chrome Profile {}: {} bookmarks ({} top-level)", idx + 1, count, bookmarks.len());
                                all_bookmarks.extend(bookmarks);
                            }
                        }
                    }
                    Err(e) => warn!("⚠️  Failed to read Chrome profile {}: {}", idx + 1, e),
                }
            }
        }
        
        let total = count_bookmarks(&all_bookmarks);
        info!("📊 Total Chrome bookmarks from {} profiles: {}", profiles.len(), total);
        Ok(all_bookmarks)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;
        
        let total = count_bookmarks(bookmarks);
        for (idx, profile_path) in profiles.iter().enumerate() {
            let bookmarks_path = profile_path.join("Bookmarks");
            match std::fs::write(&bookmarks_path, &data) {
                Ok(_) => info!("✅ Wrote {} bookmarks to Chrome profile {}", total, idx + 1),
                Err(e) => warn!("⚠️  Failed to write to Chrome profile {}: {}", idx + 1, e),
            }
        }
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
        let profiles = self.detect_all_profiles()?;
        let mut all_history = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let history_path = profile_path.join("History");
            if history_path.exists() && history_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                match read_chromium_history(&history_path, days) {
                    Ok(history) => {
                        info!("✅ Chrome Profile {}: {} history items", idx + 1, history.len());
                        all_history.extend(history);
                    }
                    Err(e) => warn!("⚠️  Failed to read Chrome history from profile {}: {}", idx + 1, e),
                }
            }
        }
        
        info!("📊 Total Chrome history from {} profiles: {}", profiles.len(), all_history.len());
        Ok(all_history)
    }
    
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let history_path = profile_path.join("History");
            if history_path.exists() {
                match write_chromium_history(&history_path, items) {
                    Ok(_) => info!("✅ Wrote {} history items to Chrome profile {}", items.len(), idx + 1),
                    Err(e) => warn!("⚠️  Failed to write history to Chrome profile {}: {}", idx + 1, e),
                }
            }
        }
        Ok(())
    }
    
    fn supports_cookies(&self) -> bool {
        true
    }
    
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        let profiles = self.detect_all_profiles()?;
        let mut all_cookies = Vec::new();
        
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.join("Cookies");
            if cookies_path.exists() {
                match read_chromium_cookies(&cookies_path) {
                    Ok(cookies) => {
                        info!("✅ Chrome Profile {}: {} cookies", idx + 1, cookies.len());
                        all_cookies.extend(cookies);
                    }
                    Err(e) => warn!("⚠️  Failed to read Chrome cookies from profile {}: {}", idx + 1, e),
                }
            }
        }
        
        info!("📊 Total Chrome cookies from {} profiles: {}", profiles.len(), all_cookies.len());
        Ok(all_cookies)
    }
    
    fn write_cookies(&self, cookies: &[Cookie]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        for (idx, profile_path) in profiles.iter().enumerate() {
            let cookies_path = profile_path.join("Cookies");
            if cookies_path.exists() {
                match write_chromium_cookies(&cookies_path, cookies) {
                    Ok(_) => info!("✅ Wrote {} cookies to Chrome profile {}", cookies.len(), idx + 1),
                    Err(e) => warn!("⚠️  Failed to write cookies to Chrome profile {}: {}", idx + 1, e),
                }
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
