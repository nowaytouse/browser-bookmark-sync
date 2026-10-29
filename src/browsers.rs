use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::browser_utils::{atomic_write_file, copy_stable_file};

fn backup_file_unique(source: &Path) -> Result<PathBuf> {
    let parent = source
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid source path: {}", source.display()))?;
    let name = source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid source path: {}", source.display()))?
        .to_string_lossy();
    let backup = parent.join(format!("{name}.backup-{}", uuid::Uuid::new_v4()));
    if source.extension().and_then(|ext| ext.to_str()) == Some("sqlite") {
        let db = rusqlite::Connection::open_with_flags(
            source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.execute("VACUUM main INTO ?1", [backup.to_string_lossy()])?;
    } else {
        copy_stable_file(source, &backup)?;
    }
    std::fs::File::open(&backup)?.sync_all()?;
    Ok(backup)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub folder: bool,
    pub children: Vec<Self>,
    pub date_added: Option<i64>,
    pub date_modified: Option<i64>,
}

// Reserved for future cookie sync feature
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

/// ⚠️ SECURITY WARNING: Password data structure
/// This is implemented for completeness but STRONGLY DISCOURAGED.
/// Passwords are encrypted by browsers and exporting them poses severe security risks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPassword {
    /// Website URL
    pub url: String,
    /// Username or email
    pub username: String,
    /// ⚠️ ENCRYPTED password - cannot be decrypted without OS keychain access
    pub encrypted_password: Vec<u8>,
    /// When the password was created
    pub date_created: Option<i64>,
    /// When the password was last used
    pub date_last_used: Option<i64>,
    /// Number of times used
    pub times_used: i32,
}

/// ⚠️ WARNING: Extension data structure
/// Extensions contain complex local state and binary data.
/// Cross-browser extension sync is technically limited.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserExtension {
    /// Extension ID (unique identifier)
    pub id: String,
    /// Extension name
    pub name: String,
    /// Version string
    pub version: String,
    /// Whether the extension is enabled
    pub enabled: bool,
    /// Extension description
    pub description: Option<String>,
    /// Homepage URL
    pub homepage_url: Option<String>,
    /// Permissions requested by the extension
    pub permissions: Vec<String>,
    /// Installation source (store URL or local path)
    pub install_source: Option<String>,
}

// Reserved for future reading list sync feature
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrowserType {
    Waterfox,
    Safari,
    Brave,
    BraveNightly,
    Chrome,
    FirefoxNightly,
    LibreWolf,
}

impl BrowserType {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Waterfox => "Waterfox",
            Self::Safari => "Safari",
            Self::Brave => "Brave",
            Self::BraveNightly => "Brave Nightly",
            Self::Chrome => "Chrome",
            Self::FirefoxNightly => "Firefox Nightly",
            Self::LibreWolf => "LibreWolf",
        }
    }
}

fn single_profile_for_write(browser: BrowserType, profiles: &[PathBuf]) -> Result<&Path> {
    match profiles {
        [profile] => Ok(profile.as_path()),
        [] => anyhow::bail!("No {} profiles found", browser.name()),
        _ => anyhow::bail!(
            "Refusing to write {} bookmarks: {} profiles were found but this adapter cannot target one profile exactly",
            browser.name(),
            profiles.len()
        ),
    }
}

pub trait BrowserAdapter: Send + Sync {
    fn browser_type(&self) -> BrowserType;
    fn detect_bookmark_path(&self) -> Result<PathBuf>;
    fn read_bookmarks(&self) -> Result<Vec<Bookmark>>;

    /// Read bookmarks from all user profiles of this browser
    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        self.read_bookmarks()
    }
    /// Read bookmarks from all user profiles with profile names
    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        let b = self.read_all_bookmarks()?;
        Ok(vec![("Default".to_string(), b)])
    }
    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()>;
    fn backup_bookmarks(&self) -> Result<PathBuf>;
    fn validate_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<bool>;

    // Reading list support (reserved for future)
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

    // Cookies support (reserved for future)
    fn supports_cookies(&self) -> bool {
        false
    }
    fn read_cookies(&self) -> Result<Vec<Cookie>> {
        Ok(vec![])
    }
    fn write_cookies(&self, _cookies: &[Cookie]) -> Result<()> {
        Ok(())
    }

    // ⚠️ Password support - SECURITY SENSITIVE
    // Implemented for completeness but STRONGLY DISCOURAGED
    fn supports_passwords(&self) -> bool {
        false
    }
    /// ⚠️ WARNING: Passwords are encrypted and cannot be decrypted without OS keychain
    fn read_passwords(&self) -> Result<Vec<SavedPassword>> {
        Ok(vec![])
    }

    // ⚠️ Extension support - LIMITED FUNCTIONALITY
    // Extensions have complex local state that cannot be fully transferred
    fn supports_extensions(&self) -> bool {
        false
    }
    /// ⚠️ WARNING: Only metadata is exported, not the actual extension files
    fn read_extensions(&self) -> Result<Vec<BrowserExtension>> {
        Ok(vec![])
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
        Box::new(LibreWolfAdapter),
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
        let valid = profiles
            .iter()
            .find(|p| p.exists() && p.metadata().is_ok_and(|m| m.len() > 0))
            .or_else(|| profiles.first());
        Ok(valid
            .ok_or_else(|| anyhow::anyhow!("No Waterfox profiles found"))?
            .clone())
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from default profile (first one with valid data)
        let profiles = self.detect_all_profiles()?;

        for profile_path in &profiles {
            match read_firefox_bookmarks(profile_path) {
                Ok(bookmarks) if !bookmarks.is_empty() => {
                    // 🔧 FIX: Use recursive count, not just top-level count
                    let count = count_bookmarks(&bookmarks);
                    info!("✅ Waterfox (Default): {} bookmarks", count);
                    return Ok(bookmarks);
                }
                Ok(_) => continue,  // Empty profile, try next
                Err(_) => continue, // Failed, try next
            }
        }

        Ok(Vec::new())
    }

    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.read_all_bookmarks_with_profiles()?;
        Ok(profiles.into_iter().flat_map(|(_, b)| b).collect())
    }

    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        let profiles = self.detect_all_profiles()?;
        read_all_firefox_bookmarks_with_profiles("Waterfox", &profiles)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let profile = single_profile_for_write(self.browser_type(), &profiles)?;
        anyhow::ensure!(
            profile.exists() && profile.metadata().is_ok_and(|m| m.len() > 0),
            "No valid Waterfox profile found"
        );

        write_firefox_bookmarks(profile, bookmarks)?;
        info!("✅ Wrote {} bookmarks to Waterfox", bookmarks.len());
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
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

        for profile_path in &profiles {
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
        let valid_profile = profiles
            .iter()
            .find(|p| p.exists() && p.metadata().is_ok_and(|m| m.len() > 0))
            .ok_or_else(|| anyhow::anyhow!("No valid Waterfox profile found"))?;

        match write_firefox_history(valid_profile, items) {
            Ok(()) => info!(
                "✅ Wrote {} history items to Waterfox (Default)",
                items.len()
            ),
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

        for profile_path in &profiles {
            let cookies_path = profile_path
                .parent()
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

        for profile_path in &profiles {
            let cookies_path = profile_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile path"))?
                .join("cookies.sqlite");

            if cookies_path.exists() {
                match write_firefox_cookies(&cookies_path, cookies) {
                    Ok(()) => {
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
                "{home}/Library/Application Support/Waterfox/Profiles"
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
            let path = PathBuf::from(format!("{home}/Library/Safari/Bookmarks.plist"));

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

    fn write_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let path = self.detect_bookmark_path()?;
            // Backup first
            self.backup_bookmarks()?;

            // Preserve Safari-owned data such as Reading List and History while
            // replacing only the bookmark nodes managed by this adapter.
            let existing_data = std::fs::read(&path)?;
            let existing: plist::Value = plist::from_bytes(&existing_data)?;
            let plist_value = bookmarks_to_safari_plist(&existing, _bookmarks)?;
            let mut data = Vec::new();
            plist::to_writer_xml(&mut data, &plist_value)?;
            atomic_write_file(&path, &data)?;

            debug!("Wrote {} bookmarks to Safari", _bookmarks.len());
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
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
            let path = PathBuf::from(format!("{home}/Library/Safari/Bookmarks.plist"));

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

    fn write_reading_list(&self, _items: &[ReadingListItem]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!("{home}/Library/Safari/Bookmarks.plist"));

            // Backup first
            self.backup_bookmarks()?;

            // Read existing plist
            let data = std::fs::read(&path)?;
            let mut plist_value: plist::Value = plist::from_bytes(&data)?;

            // Update reading list section
            update_safari_reading_list(&mut plist_value, _items)?;

            // Write back
            let mut data = Vec::new();
            plist::to_writer_xml(&mut data, &plist_value)?;
            atomic_write_file(&path, &data)?;

            info!("Wrote {} reading list items to Safari", _items.len());
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

    fn read_history(&self, _days: Option<i32>) -> Result<Vec<HistoryItem>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let history_path = PathBuf::from(format!("{home}/Library/Safari/History.db"));

            if !history_path.exists() {
                anyhow::bail!("Safari history database not found");
            }

            read_safari_history(&history_path, _days)
        }

        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }

    fn write_history(&self, _items: &[HistoryItem]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let history_path = PathBuf::from(format!("{home}/Library/Safari/History.db"));

            write_safari_history(&history_path, _items)
        }

        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Safari is only available on macOS")
        }
    }
}

// Helper function to detect all Chromium profiles
fn discover_chromium_profile_paths(base_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(base_dir)? {
        let path = entry?.path();
        if path.is_dir() && path.join("Bookmarks").is_file() {
            profiles.push(path);
        }
    }
    profiles.sort_by_key(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        (name != "Default", name.to_string())
    });
    Ok(profiles)
}

fn detect_chromium_profiles(browser_dir: &str) -> Result<Vec<PathBuf>> {
    let home = std::env::var("HOME")?;
    let base_dir = PathBuf::from(format!("{home}/Library/Application Support/{browser_dir}"));

    if !base_dir.exists() {
        anyhow::bail!("{browser_dir} directory not found");
    }

    let profiles = discover_chromium_profile_paths(&base_dir)?;

    if profiles.is_empty() {
        anyhow::bail!("No {browser_dir} profiles found");
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
        let bookmarks_path = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?
            .join("Bookmarks");
        Ok(bookmarks_path)
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles
            .first()
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

    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.read_all_bookmarks_with_profiles()?;
        Ok(profiles.into_iter().flat_map(|(_, b)| b).collect())
    }

    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        let profiles = self.detect_all_profiles()?;
        read_all_chromium_bookmarks_with_profiles("Brave", &profiles)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let profile = single_profile_for_write(self.browser_type(), &profiles)?;

        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;

        let bookmarks_path = profile.join("Bookmarks");
        atomic_write_file(&bookmarks_path, data.as_bytes())?;

        let total = count_bookmarks(bookmarks);
        info!("✅ Wrote {} bookmarks to Brave", total);
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;

        let history_path = default_profile.join("History");
        if history_path.exists() && history_path.metadata().is_ok_and(|m| m.len() > 0) {
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;

        let history_path = default_profile.join("History");
        if history_path.exists() {
            match write_chromium_history(&history_path, items) {
                Ok(()) => info!("✅ Wrote {} history items to Brave (Default)", items.len()),
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
        let default_profile = profiles
            .first()
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave profiles found"))?;

        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match write_chromium_cookies(&cookies_path, cookies) {
                Ok(()) => info!("✅ Wrote {} cookies to Brave (Default)", cookies.len()),
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
        let bookmarks_path = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?
            .join("Bookmarks");
        Ok(bookmarks_path)
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from Default profile to avoid data duplication
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles
            .first()
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

    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.read_all_bookmarks_with_profiles()?;
        Ok(profiles.into_iter().flat_map(|(_, b)| b).collect())
    }

    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        let profiles = self.detect_all_profiles()?;
        read_all_chromium_bookmarks_with_profiles("Brave Nightly", &profiles)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let profile = single_profile_for_write(self.browser_type(), &profiles)?;

        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;

        let bookmarks_path = profile.join("Bookmarks");
        atomic_write_file(&bookmarks_path, data.as_bytes())?;

        let total = count_bookmarks(bookmarks);
        info!("✅ Wrote {} bookmarks to Brave Nightly", total);
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;

        let history_path = default_profile.join("History");
        if history_path.exists() && history_path.metadata().is_ok_and(|m| m.len() > 0) {
            match read_chromium_history(&history_path, days) {
                Ok(history) => {
                    info!(
                        "✅ Brave Nightly (Default): {} history items",
                        history.len()
                    );
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;

        let history_path = default_profile.join("History");
        if history_path.exists() {
            match write_chromium_history(&history_path, items) {
                Ok(()) => info!(
                    "✅ Wrote {} history items to Brave Nightly (Default)",
                    items.len()
                ),
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
        let default_profile = profiles
            .first()
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Brave Nightly profiles found"))?;

        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match write_chromium_cookies(&cookies_path, cookies) {
                Ok(()) => info!(
                    "✅ Wrote {} cookies to Brave Nightly (Default)",
                    cookies.len()
                ),
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
        let bookmarks_path = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?
            .join("Bookmarks");
        Ok(bookmarks_path)
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        // Only read from Default profile
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles
            .first()
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

    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.read_all_bookmarks_with_profiles()?;
        Ok(profiles.into_iter().flat_map(|(_, b)| b).collect())
    }

    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        let profiles = self.detect_all_profiles()?;
        read_all_chromium_bookmarks_with_profiles("Chrome", &profiles)
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let profile = single_profile_for_write(self.browser_type(), &profiles)?;

        let json = bookmarks_to_chromium_json(bookmarks)?;
        let data = serde_json::to_string_pretty(&json)?;

        let bookmarks_path = profile.join("Bookmarks");
        atomic_write_file(&bookmarks_path, data.as_bytes())?;

        let total = count_bookmarks(bookmarks);
        info!("✅ Wrote {} bookmarks to Chrome", total);
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;

        let history_path = default_profile.join("History");
        if history_path.exists() && history_path.metadata().is_ok_and(|m| m.len() > 0) {
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;

        let history_path = default_profile.join("History");
        if history_path.exists() {
            match write_chromium_history(&history_path, items) {
                Ok(()) => info!("✅ Wrote {} history items to Chrome (Default)", items.len()),
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
        let default_profile = profiles
            .first()
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
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;

        let cookies_path = default_profile.join("Cookies");
        if cookies_path.exists() {
            match write_chromium_cookies(&cookies_path, cookies) {
                Ok(()) => info!("✅ Wrote {} cookies to Chrome (Default)", cookies.len()),
                Err(e) => warn!("⚠️  Failed to write cookies to Chrome: {}", e),
            }
        }
        Ok(())
    }

    // ⚠️ Password support - SECURITY SENSITIVE
    fn supports_passwords(&self) -> bool {
        true
    }

    fn read_passwords(&self) -> Result<Vec<SavedPassword>> {
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;

        let login_data_path = default_profile.join("Login Data");
        if login_data_path.exists() {
            match read_chromium_passwords(&login_data_path) {
                Ok(passwords) => {
                    warn!(
                        "🔴 Chrome: {} password entries (ENCRYPTED - cannot be decrypted)",
                        passwords.len()
                    );
                    return Ok(passwords);
                }
                Err(e) => warn!("⚠️  Failed to read Chrome passwords: {}", e),
            }
        }
        Ok(Vec::new())
    }

    // ⚠️ Extension support - METADATA ONLY
    fn supports_extensions(&self) -> bool {
        true
    }

    fn read_extensions(&self) -> Result<Vec<BrowserExtension>> {
        let profiles = self.detect_all_profiles()?;
        let default_profile = profiles
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Chrome profiles found"))?;

        let extensions_path = default_profile.join("Extensions");
        if extensions_path.exists() {
            match read_chromium_extensions(&extensions_path) {
                Ok(extensions) => {
                    info!("✅ Chrome: {} extensions (metadata only)", extensions.len());
                    return Ok(extensions);
                }
                Err(e) => warn!("⚠️  Failed to read Chrome extensions: {}", e),
            }
        }
        Ok(Vec::new())
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
                "{home}/Library/Application Support/Firefox/Profiles"
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

    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.read_all_bookmarks_with_profiles()?;
        Ok(profiles.into_iter().flat_map(|(_, b)| b).collect())
    }

    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        if let Ok(profiles) = self.detect_all_profiles() {
            read_all_firefox_bookmarks_with_profiles("Firefox Nightly", &profiles)
        } else if let Ok(path) = self.detect_bookmark_path() {
            read_all_firefox_bookmarks_with_profiles("Firefox Nightly", &[path])
        } else {
            Ok(Vec::new())
        }
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let path = single_profile_for_write(self.browser_type(), &profiles)?;
        write_firefox_bookmarks(&path, bookmarks)
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
    }

    fn validate_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<bool> {
        Ok(true)
    }
}

impl FirefoxNightlyAdapter {
    fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{home}/Library/Application Support/Firefox/Profiles"
            ));

            if !path.exists() {
                anyhow::bail!("Firefox profile directory not found");
            }

            let mut profiles = Vec::new();
            for entry in std::fs::read_dir(&path)? {
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
                anyhow::bail!("No Firefox profiles with bookmarks found");
            }

            info!("🔍 Found {} Firefox profile(s)", profiles.len());
            Ok(profiles)
        }

        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("Firefox detection not implemented for this platform")
        }
    }
}

// LibreWolf Adapter
pub struct LibreWolfAdapter;

impl BrowserAdapter for LibreWolfAdapter {
    fn browser_type(&self) -> BrowserType {
        BrowserType::LibreWolf
    }

    fn detect_bookmark_path(&self) -> Result<PathBuf> {
        let profiles = self.detect_all_profiles()?;
        let valid = profiles
            .iter()
            .find(|p| p.exists() && p.metadata().is_ok_and(|m| m.len() > 0))
            .or_else(|| profiles.first());
        Ok(valid
            .ok_or_else(|| anyhow::anyhow!("No LibreWolf profiles found"))?
            .clone())
    }

    fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.detect_all_profiles()?;

        for profile_path in &profiles {
            match read_firefox_bookmarks(profile_path) {
                Ok(bookmarks) if !bookmarks.is_empty() => {
                    let count = count_bookmarks(&bookmarks);
                    info!("✅ LibreWolf (Default): {} bookmarks", count);
                    return Ok(bookmarks);
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }

        Ok(Vec::new())
    }

    fn read_all_bookmarks(&self) -> Result<Vec<Bookmark>> {
        let profiles = self.read_all_bookmarks_with_profiles()?;
        Ok(profiles.into_iter().flat_map(|(_, b)| b).collect())
    }

    fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
        if let Ok(profiles) = self.detect_all_profiles() {
            read_all_firefox_bookmarks_with_profiles("LibreWolf", &profiles)
        } else if let Ok(path) = self.detect_bookmark_path() {
            read_all_firefox_bookmarks_with_profiles("LibreWolf", &[path])
        } else {
            Ok(Vec::new())
        }
    }

    fn write_bookmarks(&self, bookmarks: &[Bookmark]) -> Result<()> {
        let profiles = self.detect_all_profiles()?;
        let profile = single_profile_for_write(self.browser_type(), &profiles)?;
        anyhow::ensure!(
            profile.exists() && profile.metadata().is_ok_and(|m| m.len() > 0),
            "No valid LibreWolf profile found"
        );

        write_firefox_bookmarks(profile, bookmarks)?;
        info!("✅ Wrote {} bookmarks to LibreWolf", bookmarks.len());
        Ok(())
    }

    fn backup_bookmarks(&self) -> Result<PathBuf> {
        let source = self.detect_bookmark_path()?;
        backup_file_unique(&source)
    }

    fn validate_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<bool> {
        Ok(true)
    }
}

impl LibreWolfAdapter {
    fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")?;
            let path = PathBuf::from(format!(
                "{home}/Library/Application Support/LibreWolf/Profiles"
            ));

            if !path.exists() {
                anyhow::bail!("LibreWolf profile directory not found");
            }

            let mut profiles = Vec::new();
            for entry in std::fs::read_dir(&path)? {
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
                anyhow::bail!("No LibreWolf profiles with bookmarks found");
            }

            info!("🔍 Found {} LibreWolf profile(s)", profiles.len());
            Ok(profiles)
        }

        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("LibreWolf detection not implemented for this platform")
        }
    }
}

fn read_all_chromium_bookmarks_with_profiles(
    browser_name: &str,
    profiles: &[PathBuf],
) -> Result<Vec<(String, Vec<Bookmark>)>> {
    let mut results = Vec::new();
    for profile_path in profiles {
        let bookmarks_path = profile_path.join("Bookmarks");
        if !bookmarks_path.exists() {
            continue;
        }
        let profile_name = profile_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Profile");

        let data = std::fs::read_to_string(&bookmarks_path)
            .with_context(|| format!("Failed to read {browser_name} profile {profile_name}"))?;
        let json = serde_json::from_str::<serde_json::Value>(&data).with_context(|| {
            format!("Invalid bookmark JSON in {browser_name} profile {profile_name}")
        })?;
        let bookmarks = parse_chromium_bookmarks(&json)
            .with_context(|| format!("Failed to parse {browser_name} profile {profile_name}"))?;
        if !bookmarks.is_empty() {
            let count = count_bookmarks(&bookmarks);
            info!(
                "✅ {} ({}): {} bookmarks",
                browser_name, profile_name, count
            );
            results.push((profile_name.to_string(), bookmarks));
        }
    }
    Ok(results)
}

#[allow(dead_code)]
fn read_all_chromium_bookmarks(browser_name: &str, profiles: &[PathBuf]) -> Result<Vec<Bookmark>> {
    let results = read_all_chromium_bookmarks_with_profiles(browser_name, profiles)?;
    Ok(results.into_iter().flat_map(|(_, b)| b).collect())
}

fn read_all_firefox_bookmarks_with_profiles(
    browser_name: &str,
    profiles: &[PathBuf],
) -> Result<Vec<(String, Vec<Bookmark>)>> {
    let mut results = Vec::new();
    for profile_path in profiles {
        let profile_name = profile_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("Profile");
        let bookmarks = read_firefox_bookmarks(profile_path)
            .with_context(|| format!("Failed to read {browser_name} profile {profile_name}"))?;
        if !bookmarks.is_empty() {
            let count = count_bookmarks(&bookmarks);
            info!(
                "✅ {} ({}): {} bookmarks",
                browser_name, profile_name, count
            );
            results.push((profile_name.to_string(), bookmarks));
        }
    }
    Ok(results)
}

#[allow(dead_code)]
fn read_all_firefox_bookmarks(browser_name: &str, profiles: &[PathBuf]) -> Result<Vec<Bookmark>> {
    let results = read_all_firefox_bookmarks_with_profiles(browser_name, profiles)?;
    Ok(results.into_iter().flat_map(|(_, b)| b).collect())
}

// Helper functions for Safari plist parsing
#[cfg(target_os = "macos")]
fn parse_safari_plist(value: &plist::Value) -> Result<Vec<Bookmark>> {
    let mut bookmarks = Vec::new();

    if let Some(dict) = value.as_dictionary() {
        if let Some(children) = dict.get("Children").and_then(|v| v.as_array()) {
            for child in children {
                if let Some(bookmark) = parse_safari_bookmark_node(child) {
                    // Skip Reading List and History folders
                    if bookmark.title != "com.apple.ReadingList" && bookmark.title != "History" {
                        bookmarks.push(bookmark);
                    }
                }
            }
        }
    }

    debug!("Parsed {} bookmarks from Safari plist", bookmarks.len());
    Ok(bookmarks)
}

#[cfg(target_os = "macos")]
fn parse_safari_bookmark_node(node: &plist::Value) -> Option<Bookmark> {
    let dict = node.as_dictionary()?;

    let web_bookmark_type = dict
        .get("WebBookmarkType")
        .and_then(|v| v.as_string())
        .unwrap_or("");

    let title = dict
        .get("Title")
        .and_then(|v| v.as_string())
        .or_else(|| {
            dict.get("URIDictionary")
                .and_then(|v| v.as_dictionary())
                .and_then(|d| d.get("title"))
                .and_then(|v| v.as_string())
        })
        .unwrap_or("")
        .to_string();

    let uuid = dict
        .get("WebBookmarkUUID")
        .and_then(|v| v.as_string())
        .unwrap_or("")
        .to_string();

    match web_bookmark_type {
        "WebBookmarkTypeList" => {
            // This is a folder
            let mut children = Vec::new();
            if let Some(child_array) = dict.get("Children").and_then(|v| v.as_array()) {
                for child in child_array {
                    if let Some(child_bookmark) = parse_safari_bookmark_node(child) {
                        children.push(child_bookmark);
                    }
                }
            }

            Some(Bookmark {
                id: uuid,
                title,
                url: None,
                folder: true,
                children,
                date_added: None,
                date_modified: None,
            })
        }
        "WebBookmarkTypeLeaf" => {
            // This is a bookmark
            let url = dict
                .get("URLString")
                .and_then(|v| v.as_string())
                .map(ToString::to_string);

            Some(Bookmark {
                id: uuid,
                title,
                url,
                folder: false,
                children: vec![],
                date_added: None,
                date_modified: None,
            })
        }
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn bookmarks_to_safari_plist(
    existing: &plist::Value,
    bookmarks: &[Bookmark],
) -> Result<plist::Value> {
    use plist::{Dictionary, Value};

    fn bookmark_to_safari_node(bookmark: &Bookmark) -> Value {
        let mut dict = Dictionary::new();

        let uuid = if bookmark.id.is_empty() {
            uuid::Uuid::new_v4().to_string().to_uppercase()
        } else {
            bookmark.id.clone()
        };
        dict.insert("WebBookmarkUUID".to_string(), Value::String(uuid));

        if bookmark.folder {
            // Folder
            dict.insert(
                "WebBookmarkType".to_string(),
                Value::String("WebBookmarkTypeList".to_string()),
            );
            dict.insert("Title".to_string(), Value::String(bookmark.title.clone()));

            // Convert children
            let children: Vec<Value> = bookmark
                .children
                .iter()
                .map(bookmark_to_safari_node)
                .collect();
            dict.insert("Children".to_string(), Value::Array(children));
        } else {
            // Bookmark
            dict.insert(
                "WebBookmarkType".to_string(),
                Value::String("WebBookmarkTypeLeaf".to_string()),
            );

            if let Some(ref url) = bookmark.url {
                dict.insert("URLString".to_string(), Value::String(url.clone()));

                // URIDictionary with title
                let mut uri_dict = Dictionary::new();
                uri_dict.insert("title".to_string(), Value::String(bookmark.title.clone()));
                dict.insert("URIDictionary".to_string(), Value::Dictionary(uri_dict));
            }
        }

        Value::Dictionary(dict)
    }

    let mut root = existing
        .as_dictionary()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Safari bookmark plist root is not a dictionary"))?;
    let preserved = root
        .get("Children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|node| {
            let Some(dict) = node.as_dictionary() else {
                return true;
            };
            let title = dict.get("Title").and_then(Value::as_string).unwrap_or("");
            let kind = dict
                .get("WebBookmarkType")
                .and_then(Value::as_string)
                .unwrap_or("");
            matches!(title, "com.apple.ReadingList" | "History")
                || !matches!(kind, "WebBookmarkTypeList" | "WebBookmarkTypeLeaf")
        })
        .cloned();
    let children = bookmarks
        .iter()
        .map(bookmark_to_safari_node)
        .chain(preserved)
        .collect();
    root.insert("Children".to_string(), Value::Array(children));
    Ok(Value::Dictionary(root))
}

// Helper functions for Chromium JSON parsing
const CHROMIUM_ROOT_PREFIX: &str = "__bsync_chromium_root:";
const FIREFOX_ROOT_PREFIX: &str = "__bsync_firefox_root:";

fn parse_chromium_bookmarks(json: &serde_json::Value) -> Result<Vec<Bookmark>> {
    let mut bookmarks = Vec::new();

    if let Some(roots) = json.get("roots") {
        if let Some(roots_map) = roots.as_object() {
            let mut root_keys: Vec<_> = roots_map.keys().collect();
            root_keys.sort_unstable();
            for key in root_keys {
                let root = &roots_map[key];
                let mut children = Vec::new();
                parse_chromium_node_recursive(root, &mut children)?;
                bookmarks.push(Bookmark {
                    id: format!("{CHROMIUM_ROOT_PREFIX}{key}"),
                    title: root
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(key)
                        .to_string(),
                    url: None,
                    folder: true,
                    children,
                    date_added: root
                        .get("date_added")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| value.parse().ok()),
                    date_modified: root
                        .get("date_modified")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| value.parse().ok()),
                });
            }
        } else {
            tracing::warn!("Chromium 书签 JSON 中的 'roots' 字段不是一个合法 JSON 对象");
        }
    }

    Ok(bookmarks)
}

fn parse_chromium_node_recursive(
    node: &serde_json::Value,
    bookmarks: &mut Vec<Bookmark>,
) -> Result<()> {
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            let is_folder = child.get("type").and_then(|v| v.as_str()) == Some("folder");

            let mut bookmark = Bookmark {
                id: child
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                title: child
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: child
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                folder: is_folder,
                children: vec![],
                date_added: child
                    .get("date_added")
                    .and_then(|v| v.as_str())
                    .and_then(|s| match s.parse::<i64>() {
                        Ok(val) => Some(val),
                        Err(e) => {
                            tracing::warn!(
                                "解析 Chromium 书签 date_added 时间戳 '{}' 失败: {}",
                                s,
                                e
                            );
                            None
                        }
                    }),
                date_modified: child
                    .get("date_modified")
                    .and_then(|v| v.as_str())
                    .and_then(|s| match s.parse::<i64>() {
                        Ok(val) => Some(val),
                        Err(e) => {
                            tracing::warn!(
                                "解析 Chromium 书签 date_modified 时间戳 '{}' 失败: {}",
                                s,
                                e
                            );
                            None
                        }
                    }),
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
            let children: Vec<serde_json::Value> = bookmark
                .children
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

    fn root_metadata(key: &str) -> (&'static str, &'static str, &'static str) {
        match key {
            "bookmark_bar" => ("1", "00000000-0000-4000-a000-000000000002", "Bookmarks Bar"),
            "other" => (
                "2",
                "00000000-0000-4000-a000-000000000003",
                "Other Bookmarks",
            ),
            "synced" => (
                "3",
                "00000000-0000-4000-a000-000000000004",
                "Mobile Bookmarks",
            ),
            _ => ("0", "00000000-0000-4000-a000-000000000000", "Bookmarks"),
        }
    }

    fn make_root(
        key: &str,
        marker: Option<&Bookmark>,
        children: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        let (id, guid, default_name) = root_metadata(key);
        serde_json::json!({
            "children": children,
            "date_added": marker.and_then(|b| b.date_added).unwrap_or(0).to_string(),
            "date_last_used": "0",
            "date_modified": marker.and_then(|b| b.date_modified).unwrap_or(0).to_string(),
            "guid": guid,
            "id": id,
            "name": marker.map_or(default_name, |b| b.title.as_str()),
            "type": "folder"
        })
    }

    let mut roots = serde_json::Map::new();
    let mut unassigned = Vec::new();
    for bookmark in bookmarks {
        if let Some(key) = bookmark.id.strip_prefix(CHROMIUM_ROOT_PREFIX) {
            let children = bookmark
                .children
                .iter()
                .map(|child| convert_bookmark_recursive(child, &mut id_counter))
                .collect();
            roots.insert(key.to_string(), make_root(key, Some(bookmark), children));
        } else {
            unassigned.push(convert_bookmark_recursive(bookmark, &mut id_counter));
        }
    }

    for key in ["bookmark_bar", "other", "synced"] {
        roots
            .entry(key.to_string())
            .or_insert_with(|| make_root(key, None, Vec::new()));
    }
    if !unassigned.is_empty() {
        let children = roots
            .get_mut("bookmark_bar")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|root| root.get_mut("children"))
            .and_then(serde_json::Value::as_array_mut)
            .context("Internal Chromium bookmark_bar root has no children array")?;
        children.extend(unassigned);
    }

    Ok(serde_json::json!({
        "checksum": "",
        "roots": roots,
        "version": 1
    }))
}

// Firefox SQLite helper functions
pub fn read_firefox_bookmarks(db_path: &Path) -> Result<Vec<Bookmark>> {
    use rusqlite::{Connection, OpenFlags};
    use std::collections::HashMap;

    // SQLite itself presents a consistent view of the main database and WAL. If an
    // exclusive browser lock prevents that, fail instead of stitching together files
    // copied at different moments.
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| {
        format!(
            "Firefox profile database {} is locked; close the browser and retry",
            db_path.display()
        )
    })?;

    // Read all bookmarks with parent info
    // type=1: bookmark, type=2: folder
    // parent=2: menu, parent=3: toolbar, parent=5: unfiled
    let mut stmt = conn.prepare(
        "SELECT b.id, b.title, p.url, b.dateAdded, b.lastModified, b.type, b.parent, b.position
         FROM moz_bookmarks b
         LEFT JOIN moz_places p ON b.fk = p.id
         WHERE b.type IN (1, 2) AND b.parent >= 2
         ORDER BY b.parent, b.position",
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
                title: if let Some(t) = row.get::<_, Option<String>>(1)? {
                    t
                } else {
                    tracing::debug!("Firefox 书签 ID={} 标题为 NULL，记录为空字符串占位", id);
                    String::new()
                },
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
    fn build_tree(
        id: i64,
        all_items: &mut HashMap<i64, (Bookmark, i64)>,
        children_map: &HashMap<i64, Vec<i64>>,
    ) -> Option<Bookmark> {
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

    // Keep Firefox's root assignment in-band so cleanup/sync round-trips do not
    // silently move Menu, Other, or Mobile bookmarks into the toolbar.
    let mut bookmarks = Vec::new();
    for (root_id, title) in [
        (2, "Bookmarks Menu"),
        (3, "Bookmarks Toolbar"),
        (5, "Other Bookmarks"),
        (6, "Mobile Bookmarks"),
    ] {
        let mut children = Vec::new();
        if let Some(child_ids) = children_map.get(&root_id) {
            for child_id in child_ids {
                if let Some(bookmark) = build_tree(*child_id, &mut all_items, &children_map) {
                    children.push(bookmark);
                }
            }
        }
        bookmarks.push(Bookmark {
            id: format!("{FIREFOX_ROOT_PREFIX}{root_id}"),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: None,
            date_modified: None,
        });
    }

    let total_count = count_bookmarks(&bookmarks);
    info!(
        "📚 Read {} bookmarks (tree structure) from Firefox database",
        total_count
    );
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

fn write_firefox_bookmarks(db_path: &Path, bookmarks: &[Bookmark]) -> Result<()> {
    // 使用安全事务包装器防止数据库损坏
    crate::db_safety::safe_write_transaction(db_path, |conn| {
        // Delete only descendants of bookmark roots. The tags root (4) and its
        // children are separate Firefox data and must not be erased by cleanup.
        conn.execute(
            "WITH RECURSIVE user_nodes(id) AS (
                 SELECT id FROM moz_bookmarks WHERE parent IN (2, 3, 5, 6)
                 UNION ALL
                 SELECT child.id FROM moz_bookmarks child
                 JOIN user_nodes parent ON child.parent = parent.id
             )
             DELETE FROM moz_bookmarks WHERE id IN (SELECT id FROM user_nodes)",
            [],
        )?;

        let mut root_positions = [0i32; 4];
        let now = chrono::Utc::now().timestamp_micros();

        // Recursive function to insert bookmarks with folder structure
        fn insert_bookmark_recursive(
            conn: &rusqlite::Connection,
            bookmark: &Bookmark,
            parent_id: i64,
            position: &mut i32,
            now: i64,
        ) -> Result<()> {
            let current_position = *position;
            *position += 1;

            if bookmark.folder {
                // Generate a unique GUID for the folder
                let guid = format!(
                    "folder_{}",
                    &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
                );

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
                let folder_id: i64 =
                    conn.query_row("SELECT last_insert_rowid()", [], |row| row.get(0))?;

                // Insert children
                let mut child_position = 0i32;
                for child in &bookmark.children {
                    insert_bookmark_recursive(conn, child, folder_id, &mut child_position, now)?;
                }
            } else if let Some(url) = &bookmark.url {
                // Generate a unique GUID for the bookmark
                let guid = format!(
                    "bkmk_{}",
                    &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
                );

                // First, ensure the URL exists in moz_places
                conn.execute(
                    "INSERT OR IGNORE INTO moz_places (url, title, rev_host, hidden, typed, frecency, guid)
                     VALUES (?1, ?2, '', 0, 0, -1, ?3)",
                    rusqlite::params![url, &bookmark.title, format!("place_{}", guid)],
                )?;

                // Get the place_id
                let place_id: i64 =
                    conn.query_row("SELECT id FROM moz_places WHERE url = ?1", [url], |row| {
                        row.get(0)
                    })?;

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

        fn root_slot(root_id: i64) -> Option<usize> {
            match root_id {
                2 => Some(0),
                3 => Some(1),
                5 => Some(2),
                6 => Some(3),
                _ => None,
            }
        }

        for bookmark in bookmarks {
            if let Some(root_id) = bookmark.id.strip_prefix(FIREFOX_ROOT_PREFIX) {
                let root_id: i64 = root_id
                    .parse()
                    .with_context(|| format!("Invalid Firefox root marker: {}", bookmark.id))?;
                let slot = root_slot(root_id).with_context(|| {
                    format!(
                        "Refusing to write unknown Firefox root marker: {}",
                        bookmark.id
                    )
                })?;
                for child in &bookmark.children {
                    insert_bookmark_recursive(
                        conn,
                        child,
                        root_id,
                        &mut root_positions[slot],
                        now,
                    )?;
                }
            } else {
                // Imported data without an internal root marker uses the toolbar,
                // matching the historical behavior without disturbing marked roots.
                insert_bookmark_recursive(conn, bookmark, 3, &mut root_positions[1], now)?;
            }
        }

        let total = count_bookmarks(bookmarks);
        info!(
            "📚 Wrote {} bookmarks (tree structure) to Firefox database",
            total
        );
        Ok(())
    })
}

// Firefox history helper functions
fn read_firefox_history(db_path: &Path, days: Option<i32>) -> Result<Vec<HistoryItem>> {
    use rusqlite::{Connection, OpenFlags};

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut history = Vec::new();

    // Calculate timestamp for filtering (microseconds since epoch)
    let cutoff_timestamp = if let Some(days) = days {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(i64::from(days));
        Some(cutoff.timestamp_micros())
    } else {
        None
    };

    let query = if let Some(cutoff) = cutoff_timestamp {
        format!(
            "SELECT p.url, p.title, p.visit_count, MAX(v.visit_date) as last_visit
             FROM moz_places p
             JOIN moz_historyvisits v ON p.id = v.place_id
             WHERE v.visit_date > {cutoff}
             GROUP BY p.id
             ORDER BY last_visit DESC"
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

fn write_firefox_history(db_path: &Path, items: &[HistoryItem]) -> Result<()> {
    // 使用安全事务包装器
    crate::db_safety::safe_write_transaction(db_path, |conn| {
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

        debug!("Wrote {} history items to Firefox database", items.len());
        Ok(())
    })
}

// Profile snapshots can contain Safari plists on any platform.
pub(crate) fn parse_safari_reading_list(value: &plist::Value) -> Result<Vec<ReadingListItem>> {
    fn parse_item(value: &plist::Value) -> Option<ReadingListItem> {
        let dict = value.as_dictionary()?;
        dict.get("ReadingList")?;
        let url = dict.get("URLString")?.as_string()?;
        let title = dict
            .get("URIDictionary")
            .and_then(plist::Value::as_dictionary)
            .and_then(|uri| uri.get("title"))
            .and_then(plist::Value::as_string)
            .unwrap_or(url)
            .to_string();
        let date_added = dict
            .get("ReadingListDateAdded")
            .and_then(plist::Value::as_date)
            .and_then(|date| {
                date.to_xml_format()
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .ok()
            })
            .map(|date| date.timestamp());

        Some(ReadingListItem {
            url: url.to_string(),
            title,
            date_added,
        })
    }

    let mut items = Vec::new();
    let children = value
        .as_dictionary()
        .and_then(|dict| dict.get("Children"))
        .and_then(plist::Value::as_array)
        .into_iter()
        .flatten();
    for child in children {
        let child_dict = match child.as_dictionary() {
            Some(dict) => dict,
            None => continue,
        };
        if child_dict.get("Title").and_then(plist::Value::as_string)
            == Some("com.apple.ReadingList")
        {
            if let Some(reading_children) =
                child_dict.get("Children").and_then(plist::Value::as_array)
            {
                items.extend(reading_children.iter().filter_map(parse_item));
            }
        } else if let Some(item) = parse_item(child) {
            // Older Safari files may store ReadingList leaves at the top level.
            items.push(item);
        }
    }

    Ok(items)
}

#[cfg(target_os = "macos")]
pub(crate) fn update_safari_reading_list(
    plist: &mut plist::Value,
    items: &[ReadingListItem],
) -> Result<()> {
    use plist::{Dictionary, Value};

    let children = plist
        .as_dictionary_mut()
        .and_then(|root| root.get_mut("Children"))
        .and_then(Value::as_array_mut)
        .context("Safari Bookmarks.plist has no Children array")?;
    let reading_list = children
        .iter_mut()
        .find(|child| {
            child
                .as_dictionary()
                .and_then(|dict| dict.get("Title"))
                .and_then(Value::as_string)
                == Some("com.apple.ReadingList")
        })
        .and_then(Value::as_dictionary_mut)
        .context("Safari Bookmarks.plist has no com.apple.ReadingList folder")?;

    let values = items
        .iter()
        .map(|item| {
            let mut uri = Dictionary::new();
            uri.insert("title".into(), Value::String(item.title.clone()));

            let mut value = Dictionary::new();
            value.insert("ReadingList".into(), Value::Dictionary(Dictionary::new()));
            value.insert("URLString".into(), Value::String(item.url.clone()));
            value.insert("URIDictionary".into(), Value::Dictionary(uri));
            value.insert(
                "WebBookmarkType".into(),
                Value::String("WebBookmarkTypeLeaf".into()),
            );
            value.insert(
                "WebBookmarkUUID".into(),
                Value::String(uuid::Uuid::new_v4().to_string().to_uppercase()),
            );
            if let Some(date) = item
                .date_added
                .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                .and_then(|date| plist::Date::from_xml_format(&date.to_rfc3339()).ok())
            {
                value.insert("ReadingListDateAdded".into(), Value::Date(date));
            }
            Value::Dictionary(value)
        })
        .collect();
    reading_list.insert("Children".into(), Value::Array(values));
    Ok(())
}

// Explicit, logged epoch calculations
fn chromium_epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::NaiveDate::from_ymd_opt(1601, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or_else(
            || {
                warn!("无法构造 Chromium 纪元时间 (1601-01-01)，显式回退至 UNIX_EPOCH");
                chrono::DateTime::UNIX_EPOCH
            },
            |dt| dt.and_utc(),
        )
}

fn safari_epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::NaiveDate::from_ymd_opt(2001, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or_else(
            || {
                warn!("无法构造 Safari 纪元时间 (2001-01-01)，显式回退至 UNIX_EPOCH");
                chrono::DateTime::UNIX_EPOCH
            },
            |dt| dt.and_utc(),
        )
}

// Chromium history helper functions
fn read_chromium_history(db_path: &Path, days: Option<i32>) -> Result<Vec<HistoryItem>> {
    use rusqlite::{Connection, OpenFlags};

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut history = Vec::new();

    // Calculate timestamp for filtering (microseconds since epoch for Chromium)
    let cutoff_timestamp = if let Some(days) = days {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(i64::from(days));
        // Chromium uses microseconds since 1601-01-01
        let chromium_epoch = chromium_epoch();
        let duration = cutoff.signed_duration_since(chromium_epoch);
        Some(duration.num_microseconds().unwrap_or(0))
    } else {
        None
    };

    let query = if let Some(cutoff) = cutoff_timestamp {
        format!(
            "SELECT url, title, visit_count, last_visit_time
             FROM urls
             WHERE last_visit_time > {cutoff}
             ORDER BY last_visit_time DESC"
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

    debug!(
        "Read {} history items from Chromium database",
        history.len()
    );
    Ok(history)
}

fn write_chromium_history(db_path: &Path, items: &[HistoryItem]) -> Result<()> {
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
        let url_id: i64 =
            conn.query_row("SELECT id FROM urls WHERE url = ?1", [&item.url], |row| {
                row.get(0)
            })?;

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
fn read_safari_history(db_path: &Path, days: Option<i32>) -> Result<Vec<HistoryItem>> {
    use rusqlite::{Connection, OpenFlags};

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut history = Vec::new();

    // Safari uses Core Data timestamp (seconds since 2001-01-01)
    let cutoff_timestamp = if let Some(days) = days {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(i64::from(days));
        // Convert to Safari timestamp (seconds since 2001-01-01 00:00:00 UTC)
        let safari_epoch = safari_epoch();
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
             WHERE v.visit_time > {cutoff}
             GROUP BY i.id
             ORDER BY last_visit DESC"
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
        let safari_epoch = safari_epoch();
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

fn write_safari_history(db_path: &Path, items: &[HistoryItem]) -> Result<()> {
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
            let safari_epoch = safari_epoch();
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

// Firefox/Waterfox cookies helper functions (reserved for future)
pub(crate) fn read_firefox_cookies(db_path: &Path) -> Result<Vec<Cookie>> {
    use rusqlite::{Connection, OpenFlags};

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut cookies = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT host, name, value, path, expiry, isSecure, isHttpOnly
         FROM moz_cookies
         ORDER BY host",
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

pub(crate) fn write_firefox_cookies(db_path: &Path, cookies: &[Cookie]) -> Result<()> {
    // 使用安全事务包装器
    crate::db_safety::safe_write_transaction(db_path, |conn| {
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
                    i32::from(cookie.is_secure),
                    i32::from(cookie.is_http_only),
                ],
            )?;
        }

        debug!("Wrote {} cookies to Firefox database", cookies.len());
        Ok(())
    })
}

// Chromium cookies helper functions (reserved for future)
pub(crate) fn read_chromium_cookies(db_path: &Path) -> Result<Vec<Cookie>> {
    use rusqlite::{Connection, OpenFlags};

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut cookies = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT host_key, name, value, path, expires_utc, is_secure, is_httponly
         FROM cookies
         ORDER BY host_key",
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

pub(crate) fn write_chromium_cookies(db_path: &Path, cookies: &[Cookie]) -> Result<()> {
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
                i32::from(cookie.is_secure),
                i32::from(cookie.is_http_only),
                now,
                now,
            ],
        )?;
    }

    conn.execute("COMMIT", [])?;

    debug!("Wrote {} cookies to Chromium database", cookies.len());
    Ok(())
}

/// ⚠️ Read passwords from Chromium Login Data database
/// WARNING: Passwords are ENCRYPTED and cannot be decrypted without OS keychain access
pub(crate) fn read_chromium_passwords(db_path: &Path) -> Result<Vec<SavedPassword>> {
    use rusqlite::{Connection, OpenFlags};

    // Copy database to temp location to avoid locking issues
    let temp_dir = std::env::temp_dir();
    let temp_db = temp_dir.join("browser_sync_login_data_temp.db");
    std::fs::copy(db_path, &temp_db)?;

    let conn = Connection::open_with_flags(
        &temp_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut passwords = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT origin_url, username_value, password_value, date_created, date_last_used, times_used
         FROM logins
         ORDER BY date_last_used DESC",
    )?;

    let password_iter = stmt.query_map([], |row| {
        let encrypted_password: Vec<u8> = row.get(2)?;
        Ok(SavedPassword {
            url: row.get(0)?,
            username: row.get(1)?,
            encrypted_password,
            date_created: row.get(3)?,
            date_last_used: row.get(4)?,
            times_used: row.get(5)?,
        })
    })?;

    for password in password_iter {
        passwords.push(password?);
    }

    // Cleanup temp file
    if let Err(e) = std::fs::remove_file(&temp_db) {
        tracing::debug!("清理临时密码文件 '{}' 失败: {}", temp_db.display(), e);
    }

    debug!(
        "Read {} password entries from Chromium database (ENCRYPTED)",
        passwords.len()
    );
    Ok(passwords)
}

/// ⚠️ Read extension metadata from Chromium Extensions directory
/// WARNING: Only metadata is exported, not the actual extension files or settings
pub(crate) fn read_chromium_extensions(extensions_path: &Path) -> Result<Vec<BrowserExtension>> {
    let mut extensions = Vec::new();

    if !extensions_path.exists() {
        return Ok(extensions);
    }

    for entry in std::fs::read_dir(extensions_path)? {
        let entry = entry?;
        let ext_path = entry.path();

        if !ext_path.is_dir() {
            continue;
        }

        let ext_id = ext_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Skip temp and system directories
        if ext_id.starts_with('.') || ext_id == "Temp" {
            continue;
        }

        // Find the latest version directory
        let mut latest_version: Option<PathBuf> = None;
        if let Ok(versions) = std::fs::read_dir(&ext_path) {
            for version_entry in versions.flatten() {
                let version_path = version_entry.path();
                if version_path.is_dir() {
                    latest_version = Some(version_path);
                }
            }
        }

        if let Some(version_path) = latest_version {
            let manifest_path = version_path.join("manifest.json");
            if manifest_path.exists() {
                if let Ok(manifest_content) = std::fs::read_to_string(&manifest_path) {
                    if let Ok(manifest) =
                        serde_json::from_str::<serde_json::Value>(&manifest_content)
                    {
                        let name = manifest
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&ext_id)
                            .to_string();

                        let version = manifest
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let description = manifest
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);

                        let homepage_url = manifest
                            .get("homepage_url")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);

                        let permissions: Vec<String> = manifest
                            .get("permissions")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(ToString::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();

                        extensions.push(BrowserExtension {
                            id: ext_id,
                            name,
                            version,
                            enabled: true, // Cannot determine from filesystem alone
                            description,
                            homepage_url,
                            permissions,
                            install_source: Some("Chrome Web Store".to_string()),
                        });
                    }
                }
            }
        }
    }

    debug!(
        "Read {} extension metadata from Chromium Extensions directory",
        extensions.len()
    );
    Ok(extensions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_writes_require_exactly_one_profile() {
        let first = PathBuf::from("Profile 1");
        let second = PathBuf::from("Profile 2");

        assert_eq!(
            single_profile_for_write(BrowserType::Chrome, std::slice::from_ref(&first)).unwrap(),
            first.as_path()
        );
        assert!(single_profile_for_write(BrowserType::Chrome, &[]).is_err());
        let error = single_profile_for_write(BrowserType::Chrome, &[first, second]).unwrap_err();
        assert!(error.to_string().contains("2 profiles"));
    }

    #[test]
    fn bookmark_file_write_is_atomic_and_backups_are_unique() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Bookmarks");
        std::fs::write(&path, b"before").unwrap();

        let first_backup = backup_file_unique(&path).unwrap();
        let second_backup = backup_file_unique(&path).unwrap();
        assert_ne!(first_backup, second_backup);
        assert_eq!(std::fs::read(&first_backup).unwrap(), b"before");

        atomic_write_file(&path, b"after").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"after");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn safari_bookmark_replacement_preserves_reading_list() {
        use plist::{Dictionary, Value};

        let node = |title: &str| {
            let mut value = Dictionary::new();
            value.insert("Title".into(), Value::String(title.into()));
            value.insert(
                "WebBookmarkType".into(),
                Value::String("WebBookmarkTypeList".into()),
            );
            Value::Dictionary(value)
        };
        let mut root = Dictionary::new();
        root.insert("Marker".into(), Value::String("keep".into()));
        root.insert(
            "Children".into(),
            Value::Array(vec![node("BookmarksBar"), node("com.apple.ReadingList")]),
        );

        let merged = bookmarks_to_safari_plist(&Value::Dictionary(root), &[]).unwrap();
        let root = merged.as_dictionary().unwrap();
        assert_eq!(root.get("Marker").and_then(Value::as_string), Some("keep"));
        let children = root.get("Children").and_then(Value::as_array).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(
            children[0]
                .as_dictionary()
                .and_then(|value| value.get("Title"))
                .and_then(Value::as_string),
            Some("com.apple.ReadingList")
        );
    }

    #[test]
    fn chromium_bookmark_round_trip_preserves_root_assignment() {
        let root = |id: &str, name: &str, url: &str| {
            serde_json::json!({
                "children": [{
                    "id": format!("item-{id}"),
                    "name": name,
                    "type": "url",
                    "url": url
                }],
                "id": id,
                "name": name,
                "type": "folder"
            })
        };
        let input = serde_json::json!({
            "roots": {
                "bookmark_bar": root("1", "Bar", "https://bar.example"),
                "other": root("2", "Other", "https://other.example"),
                "synced": root("3", "Mobile", "https://mobile.example")
            },
            "version": 1
        });

        let parsed = parse_chromium_bookmarks(&input).unwrap();
        let output = bookmarks_to_chromium_json(&parsed).unwrap();
        for (key, expected) in [
            ("bookmark_bar", "https://bar.example"),
            ("other", "https://other.example"),
            ("synced", "https://mobile.example"),
        ] {
            assert_eq!(
                output["roots"][key]["children"][0]["url"].as_str(),
                Some(expected)
            );
        }
    }

    #[test]
    fn firefox_bookmark_round_trip_preserves_roots_tags_and_empty_folders() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("places.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_places (
                 id INTEGER PRIMARY KEY,
                 url TEXT UNIQUE,
                 title TEXT,
                 rev_host TEXT,
                 hidden INTEGER,
                 typed INTEGER,
                 frecency INTEGER,
                 guid TEXT
             );
             CREATE TABLE moz_bookmarks (
                 id INTEGER PRIMARY KEY,
                 type INTEGER,
                 fk INTEGER,
                 parent INTEGER,
                 position INTEGER,
                 title TEXT,
                 dateAdded INTEGER,
                 lastModified INTEGER,
                 guid TEXT
             );
             INSERT INTO moz_bookmarks VALUES
                 (1, 2, NULL, 0, 0, 'root', 0, 0, 'root________'),
                 (2, 2, NULL, 1, 0, 'menu', 0, 0, 'menu________'),
                 (3, 2, NULL, 1, 1, 'toolbar', 0, 0, 'toolbar_____'),
                 (4, 2, NULL, 1, 2, 'tags', 0, 0, 'tags________'),
                 (5, 2, NULL, 1, 3, 'unfiled', 0, 0, 'unfiled____'),
                 (6, 2, NULL, 1, 4, 'mobile', 0, 0, 'mobile______');
             INSERT INTO moz_places VALUES
                 (1, 'https://toolbar.example', 'Toolbar', '', 0, 0, -1, 'place_toolbar'),
                 (2, 'https://menu.example', 'Menu', '', 0, 0, -1, 'place_menu___'),
                 (3, 'https://tagged.example', 'Tagged', '', 0, 0, -1, 'place_tagged_');
             INSERT INTO moz_bookmarks VALUES
                 (7, 1, 1, 3, 0, 'Toolbar', 1, 1, 'bookmark_tb_'),
                 (8, 1, 2, 2, 0, 'Menu', 1, 1, 'bookmark_mn_'),
                 (9, 2, NULL, 4, 0, 'KeepTag', 1, 1, 'tag_folder__'),
                 (10, 1, 3, 9, 0, 'Tagged', 1, 1, 'tag_item____'),
                 (11, 2, NULL, 6, 0, 'Empty Mobile Folder', 1, 1, 'empty_mobile');",
        )
        .unwrap();
        drop(conn);

        let bookmarks = read_firefox_bookmarks(&db).unwrap();
        write_firefox_bookmarks(&db, &bookmarks).unwrap();

        let conn = rusqlite::Connection::open(&db).unwrap();
        for (url, parent) in [("https://toolbar.example", 3), ("https://menu.example", 2)] {
            let actual: i64 = conn
                .query_row(
                    "SELECT b.parent FROM moz_bookmarks b JOIN moz_places p ON p.id = b.fk WHERE p.url = ?1 AND b.parent != 9",
                    [url],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(actual, parent);
        }
        let tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM moz_bookmarks WHERE id IN (9, 10)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tag_count, 2);
        let mobile_folder_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM moz_bookmarks WHERE parent = 6 AND type = 2 AND title = 'Empty Mobile Folder'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mobile_folder_count, 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn safari_reading_list_round_trip_updates_nested_folder_only() {
        use plist::{Dictionary, Value};

        let reading_item = |url: &str, title: &str| {
            let mut uri = Dictionary::new();
            uri.insert("title".into(), Value::String(title.into()));
            let mut item = Dictionary::new();
            item.insert("ReadingList".into(), Value::Dictionary(Dictionary::new()));
            item.insert("URLString".into(), Value::String(url.into()));
            item.insert("URIDictionary".into(), Value::Dictionary(uri));
            Value::Dictionary(item)
        };
        let mut folder = Dictionary::new();
        folder.insert(
            "Title".into(),
            Value::String("com.apple.ReadingList".into()),
        );
        folder.insert(
            "Children".into(),
            Value::Array(vec![reading_item("https://old.example", "Old")]),
        );
        let mut other = Dictionary::new();
        other.insert("Title".into(), Value::String("BookmarksMenu".into()));
        let mut root = Dictionary::new();
        root.insert("Marker".into(), Value::String("keep".into()));
        root.insert(
            "Children".into(),
            Value::Array(vec![Value::Dictionary(other), Value::Dictionary(folder)]),
        );
        let mut plist = Value::Dictionary(root);

        assert_eq!(parse_safari_reading_list(&plist).unwrap().len(), 1);
        update_safari_reading_list(
            &mut plist,
            &[ReadingListItem {
                url: "https://new.example".into(),
                title: "New".into(),
                date_added: Some(1_700_000_000),
            }],
        )
        .unwrap();

        let parsed = parse_safari_reading_list(&plist).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].url, "https://new.example");
        assert_eq!(parsed[0].title, "New");
        assert_eq!(parsed[0].date_added, Some(1_700_000_000));
        assert_eq!(
            plist
                .as_dictionary()
                .and_then(|root| root.get("Marker"))
                .and_then(Value::as_string),
            Some("keep")
        );
    }

    #[test]
    fn chromium_profile_discovery_accepts_custom_names() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["Default", "Work Account", "Person 7"] {
            let profile = temp.path().join(name);
            std::fs::create_dir(&profile).unwrap();
            std::fs::write(profile.join("Bookmarks"), b"{}").unwrap();
        }
        std::fs::create_dir(temp.path().join("System Profile")).unwrap();

        let profiles = discover_chromium_profile_paths(temp.path()).unwrap();
        let names: Vec<_> = profiles
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["Default", "Person 7", "Work Account"]);
    }

    #[test]
    fn corrupt_chromium_profile_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("Broken");
        std::fs::create_dir(&profile).unwrap();
        std::fs::write(profile.join("Bookmarks"), b"not json").unwrap();

        let error = read_all_chromium_bookmarks_with_profiles("Test", &[profile]).unwrap_err();
        assert!(error.to_string().contains("Invalid bookmark JSON"));
    }

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
        assert_eq!(adapters.len(), 7);

        let names: Vec<&str> = adapters.iter().map(|a| a.browser_type().name()).collect();
        assert!(names.contains(&"Waterfox"));
        assert!(names.contains(&"Safari"));
        assert!(names.contains(&"Brave"));
        assert!(names.contains(&"Brave Nightly"));
        assert!(names.contains(&"Chrome"));
        assert!(names.contains(&"Firefox Nightly"));
        assert!(names.contains(&"LibreWolf"));
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
            children: vec![Bookmark {
                id: "deep".to_string(),
                title: "Deep Bookmark".to_string(),
                url: Some("https://deep.com".to_string()),
                folder: false,
                children: vec![],
                date_added: None,
                date_modified: None,
            }],
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
