pub mod collector;
pub mod dedupe;
pub mod exporter;
mod firefox_backup;
pub mod reading_list;

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

use crate::browser_utils::{
    file_fingerprint, optimistic_write_guard, FileFingerprint, OptimisticWriteGuard,
};
use crate::browsers::{
    get_all_adapters, Bookmark, BrowserAdapter, BrowserType, Cookie, HistoryItem, ReadingListItem,
};
use crate::data_types::manifest::InventoryReport;
use crate::data_types::models::{BookmarkNode, BookmarkTree};
use crate::validator::ValidationReport;

/// Configuration for hub browser operations
#[derive(Debug, Clone, Default)]
pub struct HubConfig {
    pub hub_names: String,
    pub sync_history: bool,
    pub sync_reading_list: bool,
    pub sync_cookies: bool,
    pub clear_others: bool,
    pub dry_run: bool,
    pub verbose: bool,
}

/// Configuration for HTML export operations
#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub merge: bool,
    pub deduplicate: bool,
    pub clean_empty: bool,
    pub verbose: bool,
    /// Only export bookmarks from folders matching this name
    pub folder_filter: Option<String>,
    /// Flatten export: remove browser root folders to prevent nested imports
    pub flat: bool,
    /// Custom wrap folder name (default: "📁镜像文件夹")
    pub wrap_folder: Option<String>,
    /// Disable wrapping (default: false, wrapping is enabled by default)
    pub no_wrap: bool,
    /// Export from all user profiles of each browser
    pub all_profiles: bool,
    /// Exclude Tor onion URLs before sources are aggregated
    pub exclude_onion: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            merge: false,
            deduplicate: false,
            clean_empty: false,
            verbose: false,
            folder_filter: None,
            flat: false,
            wrap_folder: None,
            no_wrap: false,
            all_profiles: true,
            exclude_onion: true,
        }
    }
}

fn is_onion_url(raw_url: &str) -> bool {
    let parsed =
        reqwest::Url::parse(raw_url).or_else(|_| reqwest::Url::parse(&format!("http://{raw_url}")));
    parsed
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            let host = host.trim_end_matches('.');
            host == "onion" || host.ends_with(".onion")
        })
}

/// Recursively remove onion-host bookmarks while preserving all other URLs and folders.
pub fn remove_onion_bookmarks(bookmarks: &mut Vec<Bookmark>) -> usize {
    let mut removed = 0;
    bookmarks.retain_mut(|bookmark| {
        removed += remove_onion_bookmarks(&mut bookmark.children);
        if bookmark.url.as_deref().is_some_and(is_onion_url) {
            removed += 1;
            false
        } else {
            true
        }
    });
    removed
}

/// Location information for a bookmark in the tree
struct BookmarkLocation {
    path: BookmarkPath, // Vector of indices representing the path in the tree
    depth: usize,
    date_added: Option<i64>,
}

/// Path to a bookmark in the tree (sequence of indices)
type BookmarkPath = Vec<usize>;

/// Sync mode (reserved for future incremental sync feature)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Incremental sync - only sync changes since last sync
    Incremental,
}

/// Sync statistics (reserved for future sync feature)
#[derive(Debug, Default)]
pub struct SyncStats {
    pub bookmarks_synced: usize,
    pub duplicates_removed: usize,
    pub conflicts_resolved: usize,
    pub errors: usize,
}

pub struct SyncEngine {
    adapters: Vec<Box<dyn BrowserAdapter + Send + Sync>>,
    last_sync_time: Option<i64>,
}

fn atomic_copy_replace(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid restore target: {}", target.display()))?;
    let name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid restore target: {}", target.display()))?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.restore-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        std::fs::copy(source, &temp)
            .with_context(|| format!("Failed to stage restore from {}", source.display()))?;
        std::fs::File::open(&temp)?.sync_all()?;
        std::fs::rename(&temp, target)
            .with_context(|| format!("Failed to atomically restore {}", target.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn restore_bookmark_file(source: &Path, target: &Path) -> Result<()> {
    if target.extension().and_then(|extension| extension.to_str()) == Some("sqlite") {
        crate::db_safety::restore_database_snapshot(source, target)
    } else {
        atomic_copy_replace(source, target)
    }
}

impl SyncEngine {
    pub fn new() -> Result<Self> {
        let adapters = get_all_adapters();
        Ok(Self {
            adapters,
            last_sync_time: None,
        })
    }

    fn read_single_profile_for_write(
        adapter: &(dyn BrowserAdapter + Send + Sync),
        operation: &str,
    ) -> Result<(Vec<Bookmark>, Option<(PathBuf, FileFingerprint)>)> {
        let path = adapter.detect_bookmark_path()?;
        let version = if path.extension().and_then(|extension| extension.to_str()) == Some("sqlite")
        {
            None
        } else {
            Some((path.clone(), file_fingerprint(&path)?))
        };
        let mut profiles = adapter.read_all_bookmarks_with_profiles()?;
        anyhow::ensure!(
            profiles.len() <= 1,
            "Refusing to {operation} {}: {} writable profiles were found ({}) but this adapter cannot target one profile exactly",
            adapter.browser_type().name(),
            profiles.len(),
            profiles
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok((
            profiles
                .pop()
                .map(|(_, bookmarks)| bookmarks)
                .unwrap_or_default(),
            version,
        ))
    }

    fn guard_bookmark_write(
        version: Option<&(PathBuf, FileFingerprint)>,
    ) -> Result<Option<OptimisticWriteGuard>> {
        version
            .map(|(path, fingerprint)| optimistic_write_guard(path, fingerprint))
            .transpose()
    }

    /// Get Safari reading list items
    pub fn get_safari_reading_list(&self) -> Result<Vec<ReadingListItem>> {
        for adapter in &self.adapters {
            if adapter.browser_type() == BrowserType::Safari {
                return adapter.read_reading_list();
            }
        }
        Ok(vec![])
    }

    /// Get history from all browsers
    pub fn get_all_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> {
        let mut all_history = Vec::new();
        for adapter in &self.adapters {
            if adapter.supports_history() {
                match adapter.read_history(days) {
                    Ok(mut history) => {
                        debug!(
                            "Read {} history items from {}",
                            history.len(),
                            adapter.browser_type().name()
                        );
                        all_history.append(&mut history);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to read history from {}: {}",
                            adapter.browser_type().name(),
                            e
                        );
                    }
                }
            }
        }
        Ok(all_history)
    }

    /// Get cookies from all browsers
    pub fn get_all_cookies(&self) -> Result<Vec<Cookie>> {
        let mut all_cookies = Vec::new();
        for adapter in &self.adapters {
            if adapter.supports_cookies() {
                match adapter.read_cookies() {
                    Ok(mut cookies) => {
                        debug!(
                            "Read {} cookies from {}",
                            cookies.len(),
                            adapter.browser_type().name()
                        );
                        all_cookies.append(&mut cookies);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to read cookies from {}: {}",
                            adapter.browser_type().name(),
                            e
                        );
                    }
                }
            }
        }
        Ok(all_cookies)
    }

    /// Load last sync timestamp from state file
    fn load_last_sync_time(&mut self) -> Result<()> {
        let state_file = Self::get_state_file_path()?;
        if state_file.exists() {
            let content = std::fs::read_to_string(&state_file)?;
            if let Ok(timestamp) = content.trim().parse::<i64>() {
                self.last_sync_time = Some(timestamp);
                debug!("Loaded last sync time: {}", timestamp);
            }
        }
        Ok(())
    }

    /// Save current sync timestamp to state file
    fn save_sync_time(&self) -> Result<()> {
        let state_file = Self::get_state_file_path()?;
        if let Some(parent) = state_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let timestamp = chrono::Utc::now().timestamp_millis();
        crate::browser_utils::atomic_write_file(&state_file, timestamp.to_string().as_bytes())?;
        debug!("Saved sync time: {}", timestamp);
        Ok(())
    }

    /// Get state file path
    fn get_state_file_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(format!("{home}/.browser-sync/last_sync")))
    }

    pub async fn sync(
        &mut self,
        mode: SyncMode,
        dry_run: bool,
        verbose: bool,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();

        // Load last sync time for incremental mode
        if mode == SyncMode::Incremental {
            if let Err(e) = self.load_last_sync_time() {
                tracing::warn!(
                    "加载上次同步时间戳记录失败: {}，本次增量同步将回退为首次全量同步",
                    e
                );
            }
            if let Some(last_sync) = self.last_sync_time {
                info!(
                    "🔄 Incremental sync mode (last sync: {})",
                    chrono::DateTime::from_timestamp_millis(last_sync).map_or_else(
                        || "unknown".to_string(),
                        |dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()
                    )
                );
            } else {
                info!("🔄 First sync - performing full sync");
            }
        } else {
            info!("🔄 Full sync mode");
        }

        info!("🔍 Phase 1: Pre-sync validation");
        self.pre_sync_validation()?;

        info!("📖 Phase 2: Reading bookmarks from all browsers");
        let mut browser_bookmarks = HashMap::new();

        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();
            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    let count = Self::count_all_bookmarks(&bookmarks);
                    info!("✅ Read {} bookmarks from {}", count, browser_type.name());
                    browser_bookmarks.insert(browser_type, bookmarks);
                }
                Err(e) => {
                    warn!(
                        "⚠️  Failed to read bookmarks from {}: {}",
                        browser_type.name(),
                        e
                    );
                    stats.errors += 1;
                }
            }
        }

        if browser_bookmarks.is_empty() {
            error!("❌ No bookmarks could be read from any browser");
            anyhow::bail!("No bookmarks available for synchronization");
        }

        info!("🧹 Phase 3: Pre-merge deduplication (smart selection)");
        let before_dedup = browser_bookmarks
            .values()
            .map(|b| Self::count_all_bookmarks(b))
            .sum::<usize>();

        // Smart deduplication for each browser (depth > date > root)
        for (browser_type, bookmarks) in &mut browser_bookmarks {
            let before = Self::count_all_bookmarks(bookmarks);
            Self::deduplicate_bookmarks_global(bookmarks);
            let after = Self::count_all_bookmarks(bookmarks);
            let removed = before.saturating_sub(after);
            if removed > 0 && verbose {
                debug!("  {} : removed {} duplicates", browser_type.name(), removed);
            }
        }

        let after_dedup = browser_bookmarks
            .values()
            .map(|b| Self::count_all_bookmarks(b))
            .sum::<usize>();

        let dedup_count = before_dedup.saturating_sub(after_dedup);
        if dedup_count > 0 {
            info!(
                "🔄 Pre-merge: removed {} duplicates (smart selection)",
                dedup_count
            );
            stats.duplicates_removed += dedup_count;
        }

        info!("🔄 Phase 4: Merging bookmarks");
        let mut merged = self.merge_bookmarks(&browser_bookmarks, verbose)?;
        let merged_count = Self::count_all_bookmarks(&merged);
        info!("📊 Merged result: {} unique bookmarks", merged_count);

        info!("🧹 Phase 5: Post-merge deduplication (final cleanup)");
        let before_final_dedup = Self::count_all_bookmarks(&merged);
        Self::deduplicate_bookmarks_global(&mut merged);
        let after_final_dedup = Self::count_all_bookmarks(&merged);

        let final_dedup_count = before_final_dedup.saturating_sub(after_final_dedup);
        if final_dedup_count > 0 {
            info!(
                "🔄 Post-merge: removed {} duplicates (final cleanup)",
                final_dedup_count
            );
            stats.duplicates_removed += final_dedup_count;
        }

        stats.bookmarks_synced = after_final_dedup;

        // Performance summary
        if verbose {
            let total_reduction = before_dedup.saturating_sub(after_final_dedup);
            let reduction_pct = if before_dedup > 0 {
                total_reduction as f64 / before_dedup as f64 * 100.0
            } else {
                0.0
            };
            debug!(
                "📊 Total reduction: {} → {} bookmarks ({:.1}% reduction)",
                before_dedup, after_final_dedup, reduction_pct
            );
        }

        if dry_run {
            info!("🏃 Dry run mode - no changes will be made");
            self.print_sync_preview(&browser_bookmarks, &merged);
            self.print_sync_stats(&stats);
            return Ok(stats);
        }

        info!("💾 Phase 6: Creating backups");
        for adapter in &self.adapters {
            match adapter.backup_bookmarks() {
                Ok(backup_path) => {
                    info!(
                        "✅ Backup created for {}: {:?}",
                        adapter.browser_type().name(),
                        backup_path
                    );
                }
                Err(e) => {
                    warn!(
                        "⚠️  Failed to backup {}: {}",
                        adapter.browser_type().name(),
                        e
                    );
                    stats.errors += 1;
                }
            }
        }

        info!("✍️  Phase 7: Writing merged bookmarks");
        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();
            let backup_path = adapter.backup_bookmarks().with_context(|| {
                format!(
                    "Refusing to write bookmarks to {}: backup failed",
                    browser_type.name()
                )
            })?;
            info!(
                "✅ Backup created for {}: {:?}",
                browser_type.name(),
                backup_path
            );
            match adapter.write_bookmarks(&merged) {
                Ok(()) => {
                    info!("✅ Wrote bookmarks to {}", browser_type.name());
                }
                Err(e) => {
                    error!(
                        "❌ Failed to write bookmarks to {}: {}",
                        browser_type.name(),
                        e
                    );
                    stats.errors += 1;
                }
            }
        }

        info!("🔍 Phase 8: Post-sync validation");
        match self.post_sync_validation(&merged) {
            Ok(()) => {}
            Err(e) => {
                warn!("⚠️  Post-sync validation warning: {}", e);
                stats.errors += 1;
            }
        }

        // Save sync time
        if let Err(e) = self.save_sync_time() {
            warn!("⚠️  Failed to save sync time: {}", e);
        }

        self.print_sync_stats(&stats);

        Ok(stats)
    }

    fn print_sync_stats(&self, stats: &SyncStats) {
        println!("\n📊 Sync Statistics:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Bookmarks synced:     {}", stats.bookmarks_synced);
        println!("  Duplicates removed:   {}", stats.duplicates_removed);
        println!("  Conflicts resolved:   {}", stats.conflicts_resolved);
        println!("  Errors encountered:   {}", stats.errors);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }

    fn pre_sync_validation(&self) -> Result<()> {
        let mut detected = 0;

        for adapter in &self.adapters {
            match adapter.detect_bookmark_path() {
                Ok(path) => {
                    debug!(
                        "✅ {} detected at: {:?}",
                        adapter.browser_type().name(),
                        path
                    );
                    detected += 1;
                }
                Err(e) => {
                    debug!("⚠️  {} not detected: {}", adapter.browser_type().name(), e);
                }
            }
        }

        if detected == 0 {
            anyhow::bail!("No browsers detected on this system");
        }

        info!(
            "✅ Pre-sync validation passed: {} browsers detected",
            detected
        );
        Ok(())
    }

    fn post_sync_validation(&self, expected: &[Bookmark]) -> Result<()> {
        let mut validation_passed = true;
        let expected_count = Self::count_all_bookmarks(expected);
        let expected_folders = Self::count_all_folders(expected);

        info!("🔍 Validating sync results...");
        info!(
            "   Expected: {} bookmarks, {} folders",
            expected_count, expected_folders
        );

        for adapter in &self.adapters {
            let browser_name = adapter.browser_type().name();

            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    let actual_count = Self::count_all_bookmarks(&bookmarks);
                    let actual_folders = Self::count_all_folders(&bookmarks);

                    // Validate structure
                    if !adapter.validate_bookmarks(&bookmarks)? {
                        warn!("⚠️  {} : structure validation failed", browser_name);
                        validation_passed = false;
                        continue;
                    }

                    // Validate counts (allow small variance due to timing)
                    let count_diff = (actual_count as i64 - expected_count as i64).abs();
                    let folder_diff = (actual_folders as i64 - expected_folders as i64).abs();

                    if count_diff > 5 {
                        warn!(
                            "⚠️  {} : bookmark count mismatch (expected: {}, actual: {})",
                            browser_name, expected_count, actual_count
                        );
                        validation_passed = false;
                    } else if folder_diff > 2 {
                        warn!(
                            "⚠️  {} : folder count mismatch (expected: {}, actual: {})",
                            browser_name, expected_folders, actual_folders
                        );
                        validation_passed = false;
                    } else {
                        debug!(
                            "✅ {} : validation passed ({} bookmarks, {} folders)",
                            browser_name, actual_count, actual_folders
                        );
                    }

                    // Check for duplicates
                    let mut url_set = HashSet::new();
                    let mut duplicate_count = 0;
                    Self::check_duplicates_recursive(
                        &bookmarks,
                        &mut url_set,
                        &mut duplicate_count,
                    );

                    if duplicate_count > 0 {
                        warn!(
                            "⚠️  {} : found {} duplicate URLs",
                            browser_name, duplicate_count
                        );
                        validation_passed = false;
                    }
                }
                Err(e) => {
                    warn!("⚠️  Could not validate {}: {}", browser_name, e);
                    validation_passed = false;
                }
            }
        }

        if validation_passed {
            info!("✅ Post-sync validation passed");
        } else {
            warn!("⚠️  Post-sync validation completed with warnings");
        }

        Ok(())
    }

    /// Check for duplicate URLs recursively
    fn check_duplicates_recursive(
        bookmarks: &[Bookmark],
        url_set: &mut HashSet<String>,
        duplicate_count: &mut usize,
    ) {
        for bookmark in bookmarks {
            if bookmark.folder {
                Self::check_duplicates_recursive(&bookmark.children, url_set, duplicate_count);
            } else if let Some(ref url) = bookmark.url {
                let normalized = Self::normalize_url(url);
                if !url_set.insert(normalized) {
                    *duplicate_count += 1;
                }
            }
        }
    }

    fn merge_bookmarks(
        &self,
        browser_bookmarks: &HashMap<BrowserType, Vec<Bookmark>>,
        verbose: bool,
    ) -> Result<Vec<Bookmark>> {
        // Find the browser with the best folder structure (most folders + most bookmarks)
        let mut best_browser: Option<BrowserType> = None;
        let mut best_score = 0i64;

        for (browser, bookmarks) in browser_bookmarks {
            let url_count = Self::count_all_bookmarks(bookmarks);
            let folder_count = Self::count_all_folders(bookmarks);
            // Score: folders are worth 1000x more than URLs (prefer structure)
            let score = (folder_count as i64 * 1000) + url_count as i64;

            if verbose {
                debug!(
                    "Browser {} has {} bookmarks, {} folders (score: {})",
                    browser.name(),
                    url_count,
                    folder_count,
                    score
                );
            }

            info!(
                "📊 {} structure: {} URLs, {} folders",
                browser.name(),
                url_count,
                folder_count
            );

            if score > best_score {
                best_score = score;
                best_browser = Some(*browser);
            }
        }

        // Use the best browser's bookmarks as base (preserving folder structure)
        let mut merged = if let Some(browser) = best_browser {
            let bookmarks = if let Some(b) = browser_bookmarks.get(&browser) {
                b.clone()
            } else {
                tracing::error!(
                    "未找到最佳同步源浏览器 '{}' 对应的书签数据列表",
                    browser.name()
                );
                Vec::new()
            };
            let url_count = Self::count_all_bookmarks(&bookmarks);
            let folder_count = Self::count_all_folders(&bookmarks);
            info!(
                "📚 Using {} as base ({} URLs, {} folders)",
                browser.name(),
                url_count,
                folder_count
            );
            bookmarks
        } else {
            Vec::new()
        };

        // 🔧 Phase 1: Clean up empty folders and invalid names
        info!("🧹 Phase 1: Cleaning up empty folders...");
        let empty_removed = Self::cleanup_empty_folders(&mut merged);
        if empty_removed > 0 {
            info!("   Removed {} empty folders", empty_removed);
        }

        // 🔧 Phase 2: Deduplicate folder structures
        info!("🔄 Phase 2: Deduplicating folder structures...");
        let folder_dupes_removed = Self::deduplicate_folder_structures(&mut merged);
        if folder_dupes_removed > 0 {
            info!("   Removed {} duplicate folders", folder_dupes_removed);
        }

        // 🔧 Phase 3: Deduplicate bookmarks by URL
        info!("🔄 Phase 3: Deduplicating bookmarks by URL...");
        let before_count = Self::count_all_bookmarks(&merged);

        // Global deduplication - track all URLs across entire tree with smart selection
        Self::deduplicate_bookmarks_global(&mut merged);

        let after_count = Self::count_all_bookmarks(&merged);

        if before_count != after_count {
            info!(
                "   Removed {} duplicate URLs ({} → {})",
                before_count - after_count,
                before_count,
                after_count
            );
        }

        // Summary
        let total_removed = empty_removed + folder_dupes_removed + (before_count - after_count);
        if total_removed > 0 {
            info!("✨ Cleanup complete: removed {} items total", total_removed);
        }

        Ok(merged)
    }

    /// Recursively deduplicate bookmarks with smart selection
    /// Priority: 1. Deeper in folder structure, 2. Newer bookmarks, 3. Root level keeps newest
    pub(crate) fn deduplicate_bookmarks_global(bookmarks: &mut Vec<Bookmark>) {
        // Two-pass strategy:
        // Pass 1: Collect all bookmarks with their metadata
        // Pass 2: For each URL, decide which one to keep, mark others for deletion
        let mut url_map: HashMap<String, Vec<BookmarkLocation>> = HashMap::new();
        Self::collect_all_bookmarks(bookmarks, &mut url_map, 0, &[]);

        // Determine which bookmark to keep for each URL
        let mut urls_to_keep: HashMap<String, BookmarkPath> = HashMap::new();
        for (url, locations) in &url_map {
            if locations.len() > 1 {
                // Find the best bookmark according to priority rules
                if let Some(best) = Self::select_best_bookmark(locations) {
                    urls_to_keep.insert(url.clone(), best.path.clone());
                } else {
                    tracing::warn!("URL '{}' 存在多个副本但无法选择最优项", url);
                }
            }
        }

        // Pass 2: Remove duplicates based on decision
        Self::remove_duplicates_by_path(bookmarks, &urls_to_keep, &[]);
    }

    /// Collect all bookmarks with their locations and metadata
    fn collect_all_bookmarks(
        bookmarks: &[Bookmark],
        url_map: &mut HashMap<String, Vec<BookmarkLocation>>,
        depth: usize,
        parent_path: &[usize],
    ) {
        for (index, bookmark) in bookmarks.iter().enumerate() {
            if bookmark.folder {
                // Recurse into folder
                let mut current_path = parent_path.to_vec();
                current_path.push(index);
                Self::collect_all_bookmarks(&bookmark.children, url_map, depth + 1, &current_path);
            } else if let Some(ref url) = bookmark.url {
                let normalized = Self::normalize_url(url);
                let mut current_path = parent_path.to_vec();
                current_path.push(index);

                let location = BookmarkLocation {
                    path: current_path,
                    depth,
                    date_added: bookmark.date_added,
                };

                url_map.entry(normalized).or_default().push(location);
            }
        }
    }

    /// Select the best bookmark from duplicates
    /// Rules:
    /// 1. Prefer bookmarks in deeper folder structure
    /// 2. If same depth, prefer newer (larger `date_added`)
    /// 3. If depth=0 for all, prefer newest
    fn select_best_bookmark(locations: &[BookmarkLocation]) -> Option<&BookmarkLocation> {
        if locations.is_empty() {
            tracing::error!("select_best_bookmark 被传入空的书签位置列表");
            return None;
        }
        locations.iter().max_by(|a, b| {
            // Compare depth first (higher is better)
            match a.depth.cmp(&b.depth) {
                std::cmp::Ordering::Equal => {
                    // Same depth, compare date (newer is better)
                    let a_date = a.date_added.unwrap_or(0);
                    let b_date = b.date_added.unwrap_or(0);
                    a_date.cmp(&b_date)
                }
                other => other,
            }
        })
    }

    /// Remove duplicates by keeping only the specified paths
    fn remove_duplicates_by_path(
        bookmarks: &mut Vec<Bookmark>,
        urls_to_keep: &HashMap<String, BookmarkPath>,
        parent_path: &[usize],
    ) {
        // First, recursively process children
        for (index, bookmark) in bookmarks.iter_mut().enumerate() {
            if bookmark.folder && !bookmark.children.is_empty() {
                let mut current_path = parent_path.to_vec();
                current_path.push(index);
                Self::remove_duplicates_by_path(
                    &mut bookmark.children,
                    urls_to_keep,
                    &current_path,
                );
            }
        }

        // Then filter current level
        let mut indices_to_remove = Vec::new();
        for (index, bookmark) in bookmarks.iter().enumerate() {
            if !bookmark.folder {
                if let Some(ref url) = bookmark.url {
                    let normalized = Self::normalize_url(url);
                    if let Some(keep_path) = urls_to_keep.get(&normalized) {
                        // This URL has duplicates, check if this is the one to keep
                        let mut current_path = parent_path.to_vec();
                        current_path.push(index);

                        if &current_path != keep_path {
                            // This is a duplicate, mark for removal
                            indices_to_remove.push(index);
                        }
                    }
                }
            }
        }

        // Remove in reverse order to maintain indices
        for &index in indices_to_remove.iter().rev() {
            bookmarks.remove(index);
        }
    }

    /// Normalize URL for deduplication comparison
    fn normalize_url(url: &str) -> String {
        url.to_string()
    }

    pub(crate) fn count_all_folders(bookmarks: &[Bookmark]) -> usize {
        let mut count = 0;
        for b in bookmarks {
            if b.folder {
                count += 1;
                count += Self::count_all_folders(&b.children);
            }
        }
        count
    }

    pub(crate) fn count_all_bookmarks(bookmarks: &[Bookmark]) -> usize {
        let mut count = 0;
        for b in bookmarks {
            if b.folder {
                count += Self::count_all_bookmarks(&b.children);
            } else {
                count += 1;
            }
        }
        count
    }

    /// Collect folder breakdown details for reporting
    pub fn get_folder_breakdown(bookmarks: &[Bookmark]) -> Vec<serde_json::Value> {
        let mut breakdown = Vec::new();
        fn recurse(b: &Bookmark, path: &str, breakdown: &mut Vec<serde_json::Value>) {
            if b.folder {
                let current_path = if path.is_empty() {
                    b.title.clone()
                } else {
                    format!("{} / {}", path, b.title)
                };
                let count = SyncEngine::count_all_bookmarks(&b.children);
                let folder_count = SyncEngine::count_all_folders(&b.children);
                let direct_bookmarks = b.children.iter().filter(|c| !c.folder).count();
                breakdown.push(serde_json::json!({
                    "folder_name": b.title,
                    "folder_path": current_path,
                    "total_bookmarks": count,
                    "direct_bookmarks": direct_bookmarks,
                    "subfolder_count": folder_count,
                }));
                for child in &b.children {
                    recurse(child, &current_path, breakdown);
                }
            }
        }
        for b in bookmarks {
            recurse(b, "", &mut breakdown);
        }
        breakdown
    }

    /// Remove empty folders and folders with invalid names
    pub(crate) fn cleanup_empty_folders(bookmarks: &mut Vec<Bookmark>) -> usize {
        let mut removed_count = 0;

        // Recursively clean up folders
        fn cleanup_recursive(bookmarks: &mut Vec<Bookmark>, removed: &mut usize) {
            bookmarks.retain_mut(|bookmark| {
                if bookmark.folder {
                    // First, recursively clean children
                    cleanup_recursive(&mut bookmark.children, removed);

                    // Remove if empty after cleaning children
                    if bookmark.children.is_empty() {
                        debug!("Removing empty folder: {}", bookmark.title);
                        *removed += 1;
                        return false;
                    }

                    // Remove if name is "/" or empty
                    if bookmark.title == "/" || bookmark.title.trim().is_empty() {
                        debug!("Removing invalid folder name: '{}'", bookmark.title);
                        *removed += 1;
                        return false;
                    }
                }
                true
            });
        }

        cleanup_recursive(bookmarks, &mut removed_count);
        removed_count
    }

    /// Extract all bookmarks from folders matching the given name
    /// Searches recursively through all bookmarks and collects contents of matching folders
    fn extract_bookmarks_from_folder(bookmarks: &[Bookmark], folder_name: &str) -> Vec<Bookmark> {
        let mut result = Vec::new();
        let folder_name_lower = folder_name.to_lowercase();

        fn search_recursive(
            bookmarks: &[Bookmark],
            folder_name_lower: &str,
            result: &mut Vec<Bookmark>,
        ) {
            for bookmark in bookmarks {
                if bookmark.folder {
                    // Check if this folder matches the target name
                    if bookmark.title.to_lowercase().contains(folder_name_lower) {
                        // Found matching folder - collect all its contents
                        debug!(
                            "Found matching folder: \"{}\" with {} items",
                            bookmark.title,
                            bookmark.children.len()
                        );
                        // Add all children (both bookmarks and subfolders)
                        result.extend(bookmark.children.clone());
                    }
                    // Also search inside this folder for nested matching folders
                    search_recursive(&bookmark.children, folder_name_lower, result);
                }
            }
        }

        search_recursive(bookmarks, &folder_name_lower, &mut result);
        result
    }

    /// Deduplicate folder structures by signature
    fn deduplicate_folder_structures(bookmarks: &mut Vec<Bookmark>) -> usize {
        let mut removed_count = 0;

        fn get_folder_signature(bookmark: &Bookmark) -> String {
            if !bookmark.folder {
                return String::new();
            }

            // Signature: name + child count + first 3 child names
            let child_names: Vec<String> = bookmark
                .children
                .iter()
                .take(3)
                .map(|c| c.title.clone())
                .collect();

            format!(
                "{}|{}|{}",
                bookmark.title,
                bookmark.children.len(),
                child_names.join(",")
            )
        }

        fn deduplicate_recursive(bookmarks: &mut Vec<Bookmark>, removed: &mut usize) {
            // Build signature map
            let mut seen_signatures: HashMap<String, usize> = HashMap::new();
            let mut to_remove: Vec<usize> = Vec::new();

            for (idx, bookmark) in bookmarks.iter().enumerate() {
                if bookmark.folder {
                    let signature = get_folder_signature(bookmark);
                    if !signature.is_empty() {
                        use std::collections::hash_map::Entry;
                        match seen_signatures.entry(signature.clone()) {
                            Entry::Occupied(_) => {
                                // Duplicate found
                                debug!(
                                    "Found duplicate folder: {} (signature: {})",
                                    bookmark.title, signature
                                );
                                to_remove.push(idx);
                                *removed += 1;
                            }
                            Entry::Vacant(e) => {
                                e.insert(idx);
                            }
                        }
                    }
                }
            }

            // Remove duplicates (in reverse order to maintain indices)
            for idx in to_remove.iter().rev() {
                bookmarks.remove(*idx);
            }

            // Recursively process children
            for bookmark in bookmarks.iter_mut() {
                if bookmark.folder {
                    deduplicate_recursive(&mut bookmark.children, removed);
                }
            }
        }

        deduplicate_recursive(bookmarks, &mut removed_count);
        removed_count
    }

    fn hash_url(&self, url: &str) -> String {
        blake3::hash(url.as_bytes()).to_hex().to_string()
    }

    fn print_sync_preview(
        &self,
        browser_bookmarks: &HashMap<BrowserType, Vec<Bookmark>>,
        merged: &[Bookmark],
    ) {
        println!("\n📊 Sync Preview:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        for (browser, bookmarks) in browser_bookmarks {
            let url_count = Self::count_all_bookmarks(bookmarks);
            let folder_count = Self::count_all_folders(bookmarks);
            println!(
                "  {} {} URLs, {} folders",
                browser.name(),
                url_count,
                folder_count
            );
        }

        let merged_urls = Self::count_all_bookmarks(merged);
        let merged_folders = Self::count_all_folders(merged);
        println!("  ─────────────────────────────────────────");
        println!("  Merged: {merged_urls} URLs, {merged_folders} folders");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }

    pub fn validate(&self, detailed: bool) -> Result<String> {
        let mut report = ValidationReport::new();

        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();

            match adapter.detect_bookmark_path() {
                Ok(path) => {
                    report.add_browser_detected(browser_type, path);

                    match adapter.read_bookmarks() {
                        Ok(bookmarks) => {
                            report.add_bookmarks_read(browser_type, bookmarks.len());

                            if adapter.validate_bookmarks(&bookmarks)? {
                                report.add_validation_passed(browser_type);
                            } else {
                                report.add_validation_failed(
                                    browser_type,
                                    "Invalid bookmark structure",
                                );
                            }
                        }
                        Err(e) => {
                            report.add_read_error(browser_type, &e.to_string());
                        }
                    }
                }
                Err(e) => {
                    report.add_not_detected(browser_type, &e.to_string());
                }
            }
        }

        Ok(report.format(detailed))
    }

    pub fn list_browsers(&self) -> Result<()> {
        println!("\n🌐 Detected Browsers:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();
            match adapter.detect_bookmark_path() {
                Ok(path) => {
                    println!("  ✅ {}", browser_type.name());
                    println!("     Path: {path:?}");
                }
                Err(_) => {
                    println!("  ❌ {} (not detected)", browser_type.name());
                }
            }
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
        Ok(())
    }

    pub async fn sync_history(
        &mut self,
        days: Option<i32>,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
        info!("📜 Starting history synchronization");

        if let Some(d) = days {
            info!("📅 Syncing history from last {} days", d);
        } else {
            info!("📅 Syncing all history");
        }

        info!("📖 Phase 1: Reading history from all browsers");
        let mut browser_history = HashMap::new();

        for adapter in &self.adapters {
            if !adapter.supports_history() {
                debug!(
                    "{} does not support history sync",
                    adapter.browser_type().name()
                );
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.read_history(days) {
                Ok(history) => {
                    info!(
                        "✅ Read {} history items from {}",
                        history.len(),
                        browser_type.name()
                    );
                    browser_history.insert(browser_type, history);
                }
                Err(e) => {
                    warn!(
                        "⚠️  Failed to read history from {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        if browser_history.is_empty() {
            warn!("⚠️  No history could be read from any browser");
            return Ok(());
        }

        info!("🔄 Phase 2: Merging history");
        let merged = self.merge_history(&browser_history, verbose)?;
        info!("📊 Merged result: {} unique history items", merged.len());

        if dry_run {
            info!("🏃 Dry run mode - no changes will be made");
            return Ok(());
        }

        info!("✍️  Phase 3: Writing merged history");
        for adapter in &self.adapters {
            if !adapter.supports_history() {
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.write_history(&merged) {
                Ok(()) => {
                    info!("✅ Wrote history to {}", browser_type.name());
                }
                Err(e) => {
                    error!(
                        "❌ Failed to write history to {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        info!("✅ History synchronization complete");
        Ok(())
    }

    pub async fn sync_reading_list(&mut self, dry_run: bool, verbose: bool) -> Result<()> {
        info!("📚 Starting reading list synchronization");

        info!("📖 Phase 1: Reading lists from all browsers");
        let mut browser_reading_lists = HashMap::new();

        for adapter in &self.adapters {
            if !adapter.supports_reading_list() {
                debug!(
                    "{} does not support reading list sync",
                    adapter.browser_type().name()
                );
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.read_reading_list() {
                Ok(items) => {
                    info!(
                        "✅ Read {} reading list items from {}",
                        items.len(),
                        browser_type.name()
                    );
                    browser_reading_lists.insert(browser_type, items);
                }
                Err(e) => {
                    warn!(
                        "⚠️  Failed to read reading list from {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        if browser_reading_lists.is_empty() {
            warn!("⚠️  No reading lists could be read from any browser");
            return Ok(());
        }

        info!("🔄 Phase 2: Merging reading lists");
        let merged = self.merge_reading_lists(&browser_reading_lists, verbose)?;
        info!(
            "📊 Merged result: {} unique reading list items",
            merged.len()
        );

        if dry_run {
            info!("🏃 Dry run mode - no changes will be made");
            return Ok(());
        }

        info!("✍️  Phase 3: Writing merged reading lists");
        for adapter in &self.adapters {
            if !adapter.supports_reading_list() {
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.write_reading_list(&merged) {
                Ok(()) => {
                    info!("✅ Wrote reading list to {}", browser_type.name());
                }
                Err(e) => {
                    error!(
                        "❌ Failed to write reading list to {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        info!("✅ Reading list synchronization complete");
        Ok(())
    }

    fn merge_history(
        &self,
        browser_history: &HashMap<BrowserType, Vec<HistoryItem>>,
        verbose: bool,
    ) -> Result<Vec<HistoryItem>> {
        let mut merged = Vec::new();
        let mut seen_urls = HashSet::new();

        for (browser, history) in browser_history {
            if verbose {
                debug!(
                    "Processing {} history items from {}",
                    history.len(),
                    browser.name()
                );
            }

            for item in history {
                let url_hash = self.hash_url(&item.url);
                if seen_urls.insert(url_hash) {
                    merged.push(item.clone());
                } else if verbose {
                    debug!("Skipping duplicate URL: {}", item.url);
                }
            }
        }

        // Sort by last visit time (most recent first)
        merged.sort_by(|a, b| b.last_visit.unwrap_or(0).cmp(&a.last_visit.unwrap_or(0)));

        Ok(merged)
    }

    fn merge_reading_lists(
        &self,
        browser_reading_lists: &HashMap<BrowserType, Vec<ReadingListItem>>,
        verbose: bool,
    ) -> Result<Vec<ReadingListItem>> {
        let mut merged = Vec::new();
        let mut seen_urls = HashSet::new();

        for (browser, items) in browser_reading_lists {
            if verbose {
                debug!(
                    "Processing {} reading list items from {}",
                    items.len(),
                    browser.name()
                );
            }

            for item in items {
                let url_hash = self.hash_url(&item.url);
                if seen_urls.insert(url_hash) {
                    merged.push(item.clone());
                } else if verbose {
                    debug!("Skipping duplicate URL: {}", item.url);
                }
            }
        }

        // Sort by date added (most recent first)
        merged.sort_by(|a, b| b.date_added.unwrap_or(0).cmp(&a.date_added.unwrap_or(0)));

        Ok(merged)
    }

    pub async fn sync_cookies(&mut self, dry_run: bool, verbose: bool) -> Result<()> {
        info!("🍪 Starting cookies synchronization");

        info!("📖 Phase 1: Reading cookies from all browsers");
        let mut browser_cookies = HashMap::new();

        for adapter in &self.adapters {
            if !adapter.supports_cookies() {
                debug!(
                    "{} does not support cookies sync",
                    adapter.browser_type().name()
                );
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.read_cookies() {
                Ok(cookies) => {
                    info!(
                        "✅ Read {} cookies from {}",
                        cookies.len(),
                        browser_type.name()
                    );
                    browser_cookies.insert(browser_type, cookies);
                }
                Err(e) => {
                    warn!(
                        "⚠️  Failed to read cookies from {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        if browser_cookies.is_empty() {
            warn!("⚠️  No cookies could be read from any browser");
            return Ok(());
        }

        info!("🔄 Phase 2: Merging cookies");
        let merged = self.merge_cookies(&browser_cookies, verbose)?;
        info!("📊 Merged result: {} unique cookies", merged.len());

        if dry_run {
            info!("🏃 Dry run mode - no changes will be made");
            return Ok(());
        }

        info!("✍️  Phase 3: Writing merged cookies");
        for adapter in &self.adapters {
            if !adapter.supports_cookies() {
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.write_cookies(&merged) {
                Ok(()) => {
                    info!("✅ Wrote cookies to {}", browser_type.name());
                }
                Err(e) => {
                    error!(
                        "❌ Failed to write cookies to {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        info!("✅ Cookies synchronization complete");
        Ok(())
    }

    /// Sync cookies to hub browsers (Brave Nightly + Waterfox)
    /// Collects cookies from all browsers to Brave Nightly, then syncs to Waterfox
    /// Does NOT delete cookies from other browsers
    pub async fn sync_cookies_to_hub(&mut self, dry_run: bool, verbose: bool) -> Result<()> {
        info!("🍪 Starting cookies synchronization to hub browsers");
        info!("📍 Hub architecture: Brave Nightly (primary) ↔ Waterfox (secondary)");

        // Phase 1: Read cookies from all browsers
        info!("📖 Phase 1: Reading cookies from all browsers");
        let mut all_cookies = Vec::new();
        let mut browser_cookie_counts = HashMap::new();

        for adapter in &self.adapters {
            if !adapter.supports_cookies() {
                if verbose {
                    debug!(
                        "{} does not support cookies sync",
                        adapter.browser_type().name()
                    );
                }
                continue;
            }

            let browser_type = adapter.browser_type();
            match adapter.read_cookies() {
                Ok(cookies) => {
                    let count = cookies.len();
                    info!("✅ Read {} cookies from {}", count, browser_type.name());
                    browser_cookie_counts.insert(browser_type, count);
                    if verbose && count > 0 {
                        // 显示前5个cookie的域名
                        let sample: Vec<_> =
                            cookies.iter().take(5).map(|c| c.host.as_str()).collect();
                        debug!("   Sample hosts: {}", sample.join(", "));
                    }
                    all_cookies.extend(cookies);
                }
                Err(e) => {
                    warn!(
                        "⚠️  Failed to read cookies from {}: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        if all_cookies.is_empty() {
            warn!("⚠️  No cookies could be read from any browser");
            return Ok(());
        }

        // Phase 2: Deduplicate cookies (performance optimized with HashSet)
        info!("🔄 Phase 2: Merging and deduplicating cookies");
        let initial_count = all_cookies.len();

        // Use HashSet for O(1) deduplication
        let mut seen = HashSet::new();
        all_cookies.retain(|cookie| {
            let key = format!("{}:{}:{}", cookie.host, cookie.name, cookie.path);
            seen.insert(key)
        });

        let merged_count = all_cookies.len();
        let duplicates_removed = initial_count - merged_count;

        info!(
            "📊 Deduplication: {} total → {} unique ({} duplicates removed)",
            initial_count, merged_count, duplicates_removed
        );

        if dry_run {
            info!("🏃 Dry run mode - no changes will be made");
            info!("📊 Summary:");
            for (browser_type, count) in browser_cookie_counts {
                info!("  {} : {} cookies", browser_type.name(), count);
            }
            info!("  Total unique: {} cookies", merged_count);
            return Ok(());
        }

        // Phase 3: Write to Brave Nightly (primary hub)
        info!("✍️  Phase 3: Writing cookies to Brave Nightly (primary hub)");
        let brave_nightly_adapter = self
            .adapters
            .iter()
            .find(|a| a.browser_type() == BrowserType::BraveNightly);

        if let Some(adapter) = brave_nightly_adapter {
            match adapter.write_cookies(&all_cookies) {
                Ok(()) => {
                    info!("✅ Wrote {} cookies to Brave Nightly", merged_count);
                }
                Err(e) => {
                    error!("❌ Failed to write cookies to Brave Nightly: {}", e);
                    return Err(e);
                }
            }
        } else {
            warn!("⚠️  Brave Nightly not detected, skipping hub sync");
        }

        // Phase 4: Read from Brave Nightly to ensure consistency
        info!("📖 Phase 4: Reading from Brave Nightly for verification");
        let hub_cookies = if let Some(adapter) = brave_nightly_adapter {
            adapter.read_cookies()?
        } else {
            all_cookies.clone()
        };

        info!("✅ Verified {} cookies in Brave Nightly", hub_cookies.len());

        // Phase 5: Sync to Waterfox (secondary hub)
        info!("✍️  Phase 5: Syncing to Waterfox (secondary hub)");
        let waterfox_adapter = self
            .adapters
            .iter()
            .find(|a| a.browser_type() == BrowserType::Waterfox);

        if let Some(adapter) = waterfox_adapter {
            match adapter.write_cookies(&hub_cookies) {
                Ok(()) => {
                    info!("✅ Synced {} cookies to Waterfox", hub_cookies.len());
                }
                Err(e) => {
                    error!("❌ Failed to write cookies to Waterfox: {}", e);
                    return Err(e);
                }
            }
        } else {
            warn!("⚠️  Waterfox not detected, hub sync incomplete");
        }

        info!("✅ Cookies hub synchronization complete");
        info!("📊 Final state:");
        info!("  Brave Nightly: {} cookies", hub_cookies.len());
        info!("  Waterfox: {} cookies", hub_cookies.len());
        info!("  Other browsers: cookies preserved");

        Ok(())
    }

    fn merge_cookies(
        &self,
        browser_cookies: &HashMap<BrowserType, Vec<Cookie>>,
        verbose: bool,
    ) -> Result<Vec<Cookie>> {
        let mut merged = Vec::new();
        let mut seen_keys = HashSet::new();

        for (browser, cookies) in browser_cookies {
            if verbose {
                debug!(
                    "Processing {} cookies from {}",
                    cookies.len(),
                    browser.name()
                );
            }

            for cookie in cookies {
                let key = format!("{}|{}|{}", cookie.host, cookie.name, cookie.path);
                let key_hash = self.hash_url(&key);
                if seen_keys.insert(key_hash) {
                    merged.push(cookie.clone());
                } else if verbose {
                    debug!("Skipping duplicate cookie: {}:{}", cookie.host, cookie.name);
                }
            }
        }

        merged.sort_by(|a, b| a.host.cmp(&b.host));

        Ok(merged)
    }
    pub async fn import_safari_html(&mut self, html_path: &str, target: &str) -> Result<()> {
        info!("📖 Reading Safari HTML export...");

        let html_content =
            std::fs::read_to_string(html_path).context("Failed to read HTML file")?;

        let bookmarks = parse_safari_html(&html_content)?;
        info!("✅ Parsed {} bookmarks from HTML", bookmarks.len());

        if target == "all" {
            info!("📝 Writing to all browsers...");
            for adapter in &self.adapters {
                let browser_type = adapter.browser_type();
                let backup_path = adapter.backup_bookmarks().with_context(|| {
                    format!(
                        "Refusing to write to {}: backup failed",
                        browser_type.name()
                    )
                })?;
                info!(
                    "✅ Backup created for {}: {:?}",
                    browser_type.name(),
                    backup_path
                );
                match adapter.write_bookmarks(&bookmarks) {
                    Ok(()) => info!("✅ Wrote to {}", browser_type.name()),
                    Err(e) => error!("❌ Failed to write to {}: {}", browser_type.name(), e),
                }
            }
        } else {
            info!("📝 Writing to {}...", target);
            // Find specific browser
            for adapter in &self.adapters {
                if adapter
                    .browser_type()
                    .name()
                    .to_lowercase()
                    .contains(&target.to_lowercase())
                {
                    let backup_path = adapter.backup_bookmarks().with_context(|| {
                        format!(
                            "Refusing to write to {}: backup failed",
                            adapter.browser_type().name()
                        )
                    })?;
                    info!("✅ Backup created: {:?}", backup_path);
                    adapter.write_bookmarks(&bookmarks)?;
                    info!("✅ Wrote to {}", adapter.browser_type().name());
                    break;
                }
            }
        }

        Ok(())
    }

    /// Set hub browsers - migrate all data to hubs and optionally clear non-hub browsers
    pub async fn set_hub_browsers(&mut self, config: &HubConfig) -> Result<()> {
        let hub_names = &config.hub_names;
        let sync_history = config.sync_history;
        let sync_reading_list = config.sync_reading_list;
        let sync_cookies = config.sync_cookies;
        let clear_others = config.clear_others;
        let dry_run = config.dry_run;
        let verbose = config.verbose;
        // Parse hub browser names
        let hub_list: Vec<String> = hub_names
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .collect();

        info!("🎯 Hub browsers: {:?}", hub_list);

        // Categorize adapters into hubs and non-hubs
        let mut hub_adapters: Vec<&Box<dyn BrowserAdapter + Send + Sync>> = Vec::new();
        let mut non_hub_adapters: Vec<&Box<dyn BrowserAdapter + Send + Sync>> = Vec::new();

        for adapter in &self.adapters {
            let name = adapter.browser_type().name().to_lowercase();
            let is_hub = hub_list.iter().any(|h| {
                // Exact matching to avoid "brave" matching "brave nightly"
                if h == "brave-nightly" || h == "brave nightly" {
                    name.contains("brave") && name.contains("nightly")
                } else if h == "brave" {
                    name == "brave" // Exact match only
                } else if h == "waterfox" {
                    name.contains("waterfox")
                } else if h == "chrome" {
                    name == "chrome"
                } else if h == "safari" {
                    name == "safari"
                } else {
                    name.contains(h)
                }
            });

            if is_hub {
                info!("  ✅ Hub: {}", adapter.browser_type().name());
                hub_adapters.push(adapter);
            } else {
                info!("  📦 Non-hub: {}", adapter.browser_type().name());
                non_hub_adapters.push(adapter);
            }
        }

        if hub_adapters.is_empty() {
            anyhow::bail!("No hub browsers detected! Check browser names.");
        }

        // Phase 1: Read all data from all browsers
        info!("\n📖 Phase 1: Reading data from all browsers...");

        // Read bookmarks
        let mut all_bookmarks: HashMap<BrowserType, Vec<Bookmark>> = HashMap::new();
        for adapter in &self.adapters {
            if let Ok(bookmarks) = adapter.read_bookmarks() {
                let url_count = Self::count_all_bookmarks(&bookmarks);
                let folder_count = Self::count_all_folders(&bookmarks);
                info!(
                    "  {} : {} URLs, {} folders",
                    adapter.browser_type().name(),
                    url_count,
                    folder_count
                );
                all_bookmarks.insert(adapter.browser_type(), bookmarks);
            }
        }

        // Read history if requested
        let mut all_history: HashMap<BrowserType, Vec<HistoryItem>> = HashMap::new();
        if sync_history {
            info!("\n📜 Reading history...");
            for adapter in &self.adapters {
                if adapter.supports_history() {
                    if let Ok(history) = adapter.read_history(None) {
                        info!(
                            "  {} : {} history items",
                            adapter.browser_type().name(),
                            history.len()
                        );
                        all_history.insert(adapter.browser_type(), history);
                    }
                }
            }
        }

        // Read reading lists if requested
        let mut all_reading_lists: HashMap<BrowserType, Vec<ReadingListItem>> = HashMap::new();
        if sync_reading_list {
            info!("\n📚 Reading reading lists...");
            for adapter in &self.adapters {
                if adapter.supports_reading_list() {
                    if let Ok(items) = adapter.read_reading_list() {
                        info!(
                            "  {} : {} reading list items",
                            adapter.browser_type().name(),
                            items.len()
                        );
                        all_reading_lists.insert(adapter.browser_type(), items);
                    }
                }
            }
        }

        // Read cookies if requested
        let mut all_cookies: HashMap<BrowserType, Vec<Cookie>> = HashMap::new();
        if sync_cookies {
            info!("\n🍪 Reading cookies...");
            for adapter in &self.adapters {
                if adapter.supports_cookies() {
                    if let Ok(cookies) = adapter.read_cookies() {
                        info!(
                            "  {} : {} cookies",
                            adapter.browser_type().name(),
                            cookies.len()
                        );
                        all_cookies.insert(adapter.browser_type(), cookies);
                    }
                }
            }
        }

        // Phase 2: Merge and deduplicate
        info!("\n🔄 Phase 2: Merging and deduplicating...");

        let merged_bookmarks = self.merge_bookmarks(&all_bookmarks, verbose)?;
        let merged_urls = Self::count_all_bookmarks(&merged_bookmarks);
        let merged_folders = Self::count_all_folders(&merged_bookmarks);
        info!(
            "  📚 Merged bookmarks: {} URLs, {} folders",
            merged_urls, merged_folders
        );

        let merged_history = if sync_history {
            let h = self.merge_history(&all_history, verbose)?;
            info!("  📜 Merged history: {} items", h.len());
            h
        } else {
            Vec::new()
        };

        let merged_reading_list = if sync_reading_list {
            let r = self.merge_reading_lists(&all_reading_lists, verbose)?;
            info!("  📚 Merged reading list: {} items", r.len());
            r
        } else {
            Vec::new()
        };

        let merged_cookies = if sync_cookies {
            let c = self.merge_cookies(&all_cookies, verbose)?;
            info!("  🍪 Merged cookies: {} items", c.len());
            c
        } else {
            Vec::new()
        };

        if dry_run {
            info!("\n🏃 Dry run mode - no changes will be made");
            println!("\n📊 Summary (Dry Run):");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  Hub browsers will receive:");
            println!("    📚 {merged_urls} bookmarks ({merged_folders} folders)");
            if sync_history {
                println!("    📜 {} history items", merged_history.len());
            }
            if sync_reading_list {
                println!("    📖 {} reading list items", merged_reading_list.len());
            }
            if sync_cookies {
                println!("    🍪 {} cookies", merged_cookies.len());
            }
            if clear_others {
                println!("\n  Non-hub browsers will be cleared:");
                for adapter in &non_hub_adapters {
                    println!("    🗑️  {}", adapter.browser_type().name());
                }
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            return Ok(());
        }

        // Phase 3: Backup everything
        info!("\n💾 Phase 3: Creating backups...");
        for adapter in &self.adapters {
            if let Ok(path) = adapter.backup_bookmarks() {
                info!(
                    "  ✅ Backup: {} -> {:?}",
                    adapter.browser_type().name(),
                    path
                );
            }
        }

        // Phase 4: Write to hub browsers
        info!("\n✍️  Phase 4: Writing to hub browsers...");
        for adapter in &hub_adapters {
            let browser_name = adapter.browser_type().name();

            // Write bookmarks
            let backup_path = adapter
                .backup_bookmarks()
                .with_context(|| format!("Refusing to write to {browser_name}: backup failed"))?;
            info!("  ✅ Backup: {browser_name} -> {:?}", backup_path);
            match adapter.write_bookmarks(&merged_bookmarks) {
                Ok(()) => info!("  ✅ {} : bookmarks written", browser_name),
                Err(e) => error!("  ❌ {} : failed to write bookmarks: {}", browser_name, e),
            }

            // Write history
            if sync_history && adapter.supports_history() {
                match adapter.write_history(&merged_history) {
                    Ok(()) => info!("  ✅ {} : history written", browser_name),
                    Err(e) => warn!("  ⚠️  {} : failed to write history: {}", browser_name, e),
                }
            }

            // Write reading list
            if sync_reading_list && adapter.supports_reading_list() {
                match adapter.write_reading_list(&merged_reading_list) {
                    Ok(()) => info!("  ✅ {} : reading list written", browser_name),
                    Err(e) => warn!(
                        "  ⚠️  {} : failed to write reading list: {}",
                        browser_name, e
                    ),
                }
            }

            // Write cookies
            if sync_cookies && adapter.supports_cookies() {
                match adapter.write_cookies(&merged_cookies) {
                    Ok(()) => info!("  ✅ {} : cookies written", browser_name),
                    Err(e) => warn!("  ⚠️  {} : failed to write cookies: {}", browser_name, e),
                }
            }
        }

        // Phase 5: Clear non-hub browsers if requested
        if clear_others {
            info!("\n🗑️  Phase 5: Clearing non-hub browsers...");
            for adapter in &non_hub_adapters {
                let browser_name = adapter.browser_type().name();

                // Clear bookmarks by writing empty structure
                let empty_bookmarks: Vec<Bookmark> = Vec::new();
                let (_, version) =
                    Self::read_single_profile_for_write(adapter.as_ref(), "clear bookmarks from")?;
                let backup_path = adapter
                    .backup_bookmarks()
                    .with_context(|| format!("Refusing to clear {browser_name}: backup failed"))?;
                info!("  ✅ Backup: {browser_name} -> {:?}", backup_path);
                let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
                match adapter.write_bookmarks(&empty_bookmarks) {
                    Ok(()) => info!("  ✅ {} : bookmarks cleared", browser_name),
                    Err(e) => warn!("  ⚠️  {} : failed to clear bookmarks: {}", browser_name, e),
                }
            }
        }

        // Phase 6: Verification
        info!("\n🔍 Phase 6: Verification...");
        for adapter in &hub_adapters {
            if let Ok(bookmarks) = adapter.read_bookmarks() {
                let url_count = Self::count_all_bookmarks(&bookmarks);
                let folder_count = Self::count_all_folders(&bookmarks);
                info!(
                    "  ✅ {} : {} URLs, {} folders",
                    adapter.browser_type().name(),
                    url_count,
                    folder_count
                );
            }
        }

        println!("\n📊 Hub Configuration Complete!");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Hub browsers: {hub_list:?}");
        println!("  Bookmarks: {merged_urls} URLs, {merged_folders} folders");
        if sync_history {
            println!("  History: {} items synced", merged_history.len());
        }
        if sync_reading_list {
            println!("  Reading list: {} items synced", merged_reading_list.len());
        }
        if sync_cookies {
            println!("  Cookies: {} items synced", merged_cookies.len());
        }
        if clear_others {
            println!("  Non-hub browsers: CLEARED");
        }
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        Ok(())
    }

    /// Synchronize specific scenario folders across browsers
    pub async fn sync_scenario_folders(
        &mut self,
        scenario_path: &str,
        browser_names: &str,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
        info!("📁 Starting scenario folder synchronization");
        info!("🎯 Scenario path: {}", scenario_path);

        // Parse browser names
        let browser_list: Vec<String> = browser_names
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .collect();

        info!("🌐 Target browsers: {:?}", browser_list);

        // Filter adapters for specified browsers
        let mut target_adapters = Vec::new();
        for adapter in &self.adapters {
            let name = adapter.browser_type().name().to_lowercase();
            if browser_list.iter().any(|b| name.contains(b)) {
                target_adapters.push(adapter);
                info!("  ✅ {}", adapter.browser_type().name());
            }
        }

        if target_adapters.is_empty() {
            anyhow::bail!("No matching browsers found for: {browser_list:?}");
        }

        // Read scenario folders from all target browsers
        info!("\n📖 Phase 1: Reading scenario folders from browsers...");
        let mut scenario_folders: HashMap<BrowserType, Option<Bookmark>> = HashMap::new();

        for adapter in &target_adapters {
            let browser_type = adapter.browser_type();
            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    let folder = Self::find_folder_by_path(&bookmarks, scenario_path);
                    if let Some(ref f) = folder {
                        let count = Self::count_all_bookmarks(&f.children);
                        info!(
                            "  ✅ {} : found folder with {} bookmarks",
                            browser_type.name(),
                            count
                        );
                    } else {
                        info!("  ⚠️  {} : scenario folder not found", browser_type.name());
                    }
                    scenario_folders.insert(browser_type, folder);
                }
                Err(e) => {
                    warn!(
                        "  ❌ {} : failed to read bookmarks: {}",
                        browser_type.name(),
                        e
                    );
                }
            }
        }

        // Merge scenario folders
        info!("\n🔄 Phase 2: Merging scenario folders...");
        let merged_folder =
            self.merge_scenario_folders(&scenario_folders, scenario_path, verbose)?;
        let merged_count = Self::count_all_bookmarks(&merged_folder.children);
        info!("  📊 Merged folder contains {} bookmarks", merged_count);

        if dry_run {
            info!("\n🏃 Dry run mode - no changes will be made");
            println!("\n📊 Scenario Sync Preview:");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  Scenario: {scenario_path}");
            println!("  Merged bookmarks: {merged_count}");
            println!("  Target browsers:");
            for adapter in &target_adapters {
                println!("    - {}", adapter.browser_type().name());
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            return Ok(());
        }

        info!("\n✍️  Phase 4: Updating scenario folders...");
        for adapter in &target_adapters {
            match Self::read_single_profile_for_write(
                adapter.as_ref(),
                "update a scenario folder in",
            ) {
                Ok((mut bookmarks, version)) => {
                    // Replace or create scenario folder
                    if Self::replace_folder_by_path(&mut bookmarks, scenario_path, &merged_folder) {
                        let backup_path = adapter.backup_bookmarks().with_context(|| {
                            format!(
                                "Refusing to update {}: backup failed",
                                adapter.browser_type().name()
                            )
                        })?;
                        info!("  ✅ Backup: {:?}", backup_path);
                        let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
                        adapter.write_bookmarks(&bookmarks).with_context(|| {
                            format!(
                                "Failed to update scenario folder in {}",
                                adapter.browser_type().name()
                            )
                        })?;
                        info!(
                            "  ✅ {} : scenario folder updated",
                            adapter.browser_type().name()
                        );
                    } else {
                        warn!(
                            "  ⚠️  {} : failed to locate/create scenario folder",
                            adapter.browser_type().name()
                        );
                    }
                }
                Err(e) => return Err(e),
            }
        }

        info!("\n✅ Scenario folder synchronization complete!");
        Ok(())
    }

    /// Find a folder by path (e.g., "Work/Projects")
    fn find_folder_by_path(bookmarks: &[Bookmark], path: &str) -> Option<Bookmark> {
        let parts: Vec<&str> = path.split('/').collect();
        Self::find_folder_recursive(bookmarks, &parts, 0)
    }

    fn find_folder_recursive(
        bookmarks: &[Bookmark],
        parts: &[&str],
        depth: usize,
    ) -> Option<Bookmark> {
        if depth >= parts.len() {
            return None;
        }

        let target_name = parts[depth].trim().to_lowercase();

        for bookmark in bookmarks {
            if bookmark.folder && bookmark.title.to_lowercase() == target_name {
                if depth == parts.len() - 1 {
                    // Found the target folder
                    return Some(bookmark.clone());
                }
                // Continue searching in children
                return Self::find_folder_recursive(&bookmark.children, parts, depth + 1);
            }
        }

        None
    }

    /// Replace a folder at the specified path, or create it if it doesn't exist
    fn replace_folder_by_path(
        bookmarks: &mut Vec<Bookmark>,
        path: &str,
        new_folder: &Bookmark,
    ) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        Self::replace_folder_recursive(bookmarks, &parts, 0, new_folder)
    }

    fn replace_folder_recursive(
        bookmarks: &mut Vec<Bookmark>,
        parts: &[&str],
        depth: usize,
        new_folder: &Bookmark,
    ) -> bool {
        if depth >= parts.len() {
            return false;
        }

        let target_name = parts[depth].trim().to_lowercase();

        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder && bookmark.title.to_lowercase() == target_name {
                if depth == parts.len() - 1 {
                    // Replace this folder's children with new folder's children
                    bookmark.children = new_folder.children.clone();
                    return true;
                }
                // Continue searching in children
                return Self::replace_folder_recursive(
                    &mut bookmark.children,
                    parts,
                    depth + 1,
                    new_folder,
                );
            }
        }

        // If folder not found, create it at the current level
        if depth == parts.len() - 1 {
            let mut folder_to_add = new_folder.clone();
            folder_to_add.title = parts[depth].trim().to_string();
            bookmarks.push(folder_to_add);
            return true;
        }

        false
    }

    /// Merge scenario folders from multiple browsers
    fn merge_scenario_folders(
        &self,
        scenario_folders: &HashMap<BrowserType, Option<Bookmark>>,
        scenario_path: &str,
        verbose: bool,
    ) -> Result<Bookmark> {
        // Collect all valid folders
        let mut all_children = Vec::new();

        for (browser, folder_opt) in scenario_folders {
            if let Some(folder) = folder_opt {
                if verbose {
                    let count = Self::count_all_bookmarks(&folder.children);
                    debug!(
                        "  {} : {} bookmarks in scenario folder",
                        browser.name(),
                        count
                    );
                }
                all_children.extend(folder.children.clone());
            }
        }

        // Deduplicate globally with smart selection
        Self::deduplicate_bookmarks_global(&mut all_children);

        // Create merged folder
        let path_parts: Vec<&str> = scenario_path.split('/').collect();
        let folder_name = path_parts.last().unwrap_or(&"Scenario").to_string();

        Ok(Bookmark {
            id: format!("scenario-{}", chrono::Utc::now().timestamp_millis()),
            title: folder_name,
            url: None,
            folder: true,
            children: all_children,
            date_added: Some(chrono::Utc::now().timestamp_millis()),
            date_modified: Some(chrono::Utc::now().timestamp_millis()),
        })
    }

    /// Clean up duplicates and empty folders
    pub async fn cleanup_bookmarks(
        &mut self,
        browser_names: Option<&str>,
        remove_duplicates: bool,
        remove_empty_folders: bool,
        dry_run: bool,
        _verbose: bool,
    ) -> Result<()> {
        info!("🧹 Starting bookmark cleanup");

        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> =
                names.split(',').map(|s| s.trim().to_lowercase()).collect();

            self.adapters
                .iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    browser_list.iter().any(|b| name.contains(b))
                })
                .collect()
        } else {
            self.adapters.iter().collect()
        };

        if target_adapters.is_empty() {
            anyhow::bail!("No browsers found for cleanup");
        }

        info!("🎯 Target browsers:");
        for adapter in &target_adapters {
            info!("  - {}", adapter.browser_type().name());
        }

        // Process each browser
        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();

            match Self::read_single_profile_for_write(adapter.as_ref(), "clean bookmarks in") {
                Ok((mut bookmarks, version)) => {
                    let initial_count = Self::count_all_bookmarks(&bookmarks);
                    let initial_folders = Self::count_all_folders(&bookmarks);

                    info!(
                        "\n📊 {} : {} bookmarks, {} folders",
                        browser_name, initial_count, initial_folders
                    );

                    let mut stats = CleanupStats::default();

                    // Step 1: Remove duplicates with smart selection
                    if remove_duplicates {
                        Self::deduplicate_bookmarks_global(&mut bookmarks);
                        let after_dedup = Self::count_all_bookmarks(&bookmarks);
                        stats.duplicates_removed = initial_count.saturating_sub(after_dedup);

                        if stats.duplicates_removed > 0 {
                            info!(
                                "  🔄 Removed {} duplicate bookmarks",
                                stats.duplicates_removed
                            );
                        }
                    }

                    // Step 2: Remove empty folders
                    if remove_empty_folders {
                        stats.empty_folders_removed = Self::remove_empty_folders(&mut bookmarks);

                        if stats.empty_folders_removed > 0 {
                            info!(
                                "  🗑️  Removed {} empty folders",
                                stats.empty_folders_removed
                            );
                        }
                    }

                    let final_count = Self::count_all_bookmarks(&bookmarks);
                    let final_folders = Self::count_all_folders(&bookmarks);

                    if dry_run {
                        info!(
                            "  🏃 Dry run - would remove {} duplicates, {} empty folders",
                            stats.duplicates_removed, stats.empty_folders_removed
                        );
                    } else if stats.duplicates_removed > 0 || stats.empty_folders_removed > 0 {
                        let backup_path = adapter.backup_bookmarks().with_context(|| {
                            format!("Refusing to modify {browser_name}: backup failed")
                        })?;
                        info!("  💾 Backup created: {:?}", backup_path);

                        let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
                        adapter.write_bookmarks(&bookmarks).with_context(|| {
                            format!("Failed to write cleaned {browser_name} bookmarks")
                        })?;
                        info!(
                            "  ✅ Cleanup complete: {} bookmarks, {} folders remaining",
                            final_count, final_folders
                        );
                    } else {
                        info!("  ✨ No cleanup needed - bookmarks are already clean!");
                    }
                }
                Err(e) => return Err(e),
            }
        }

        info!("\n✅ Cleanup complete!");
        Ok(())
    }

    /// Organize homepage bookmarks into a dedicated folder
    /// Homepage = URL that is a root domain (e.g., `<https://example.com>` or `<https://example.com/>`)
    pub async fn organize_homepages(
        &mut self,
        browser_names: Option<&str>,
        dry_run: bool,
        _verbose: bool,
    ) -> Result<()> {
        info!("📋 Starting homepage organization");

        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> =
                names.split(',').map(|s| s.trim().to_lowercase()).collect();

            self.adapters
                .iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    browser_list.iter().any(|b| name.contains(b))
                })
                .collect()
        } else {
            self.adapters.iter().collect()
        };

        if target_adapters.is_empty() {
            anyhow::bail!("No browsers found for organization");
        }

        info!("🎯 Target browsers:");
        for adapter in &target_adapters {
            info!("  - {}", adapter.browser_type().name());
        }

        // Process each browser
        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();

            match Self::read_single_profile_for_write(adapter.as_ref(), "organize homepages in") {
                Ok((mut bookmarks, version)) => {
                    info!("\n📊 {} : Processing...", browser_name);

                    // Collect all homepages from entire tree first
                    let mut homepages_collected: Vec<Bookmark> = Vec::new();
                    Self::collect_homepages_recursive(&mut bookmarks, &mut homepages_collected);

                    let moved_count = homepages_collected.len();

                    if moved_count > 0 {
                        // Find or create "网站主页" folder at root level
                        let homepage_folder = bookmarks
                            .iter_mut()
                            .find(|b| b.folder && b.title == "网站主页");

                        if let Some(folder) = homepage_folder {
                            folder.children.extend(homepages_collected);
                        } else {
                            let new_folder = Bookmark {
                                id: format!(
                                    "homepage-folder-{}",
                                    chrono::Utc::now().timestamp_millis()
                                ),
                                title: "网站主页".to_string(),
                                url: None,
                                folder: true,
                                children: homepages_collected,
                                date_added: Some(chrono::Utc::now().timestamp_millis()),
                                date_modified: Some(chrono::Utc::now().timestamp_millis()),
                            };
                            bookmarks.push(new_folder);
                        }
                        info!(
                            "  📁 Moved {} homepage bookmarks to root \"Homepages\" folder",
                            moved_count
                        );
                    } else {
                        info!("  ✨ No homepages found to organize");
                    }

                    if dry_run {
                        info!(
                            "  🏃 Dry run - would move {} homepages to root folder",
                            moved_count
                        );
                    } else if moved_count > 0 {
                        let backup_path = adapter.backup_bookmarks().with_context(|| {
                            format!("Refusing to organize {browser_name}: backup failed")
                        })?;
                        info!("  💾 Backup created: {:?}", backup_path);

                        // Write organized bookmarks
                        let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
                        adapter
                            .write_bookmarks(&bookmarks)
                            .context("Failed to write organized bookmarks")?;
                        info!("  ✅ Organization complete");
                    }
                }
                Err(e) => return Err(e),
            }
        }

        info!("\n✅ Organization complete!");
        Ok(())
    }

    /// Recursively collect homepages from entire bookmark tree
    /// Removes homepages from their original locations and collects them
    fn collect_homepages_recursive(bookmarks: &mut Vec<Bookmark>, collected: &mut Vec<Bookmark>) {
        // First pass: recursively process children
        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder && bookmark.title != "网站主页" && !bookmark.children.is_empty()
            {
                Self::collect_homepages_recursive(&mut bookmark.children, collected);
            }
        }

        // Second pass: identify and collect homepages at current level
        let mut indices_to_remove = Vec::new();
        for (i, bookmark) in bookmarks.iter().enumerate() {
            if !bookmark.folder {
                if let Some(ref url) = bookmark.url {
                    if Self::is_homepage_url(url) {
                        collected.push(bookmark.clone());
                        indices_to_remove.push(i);
                    }
                }
            }
        }

        // Remove homepages from current level (in reverse to maintain indices)
        for &i in indices_to_remove.iter().rev() {
            bookmarks.remove(i);
        }
    }

    /// Check if a URL is a homepage (root domain)
    /// Examples: `<https://example.com>`, `<https://example.com/>`, `<http://example.com>`
    fn is_homepage_url(url: &str) -> bool {
        // Parse URL
        let normalized = url.trim().to_lowercase();

        // Must start with http:// or https://
        if !normalized.starts_with("http://") && !normalized.starts_with("https://") {
            return false;
        }

        // Remove protocol
        let without_protocol = normalized
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        // Remove trailing slash
        let without_slash = without_protocol.trim_end_matches('/');

        // Check if it's just a domain (no path)
        // Should not contain '/' after domain
        if without_slash.contains('/') {
            return false;
        }

        // Should contain at least one dot (domain.tld)
        // But allow single-word domains like http://localhost
        true
    }

    /// Recursively remove empty folders and return count of removed folders
    fn remove_empty_folders(bookmarks: &mut Vec<Bookmark>) -> usize {
        let mut removed_count = 0;

        // First, recursively clean children
        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder {
                removed_count += Self::remove_empty_folders(&mut bookmark.children);
            }
        }

        // Then remove empty folders at this level
        let before_count = bookmarks.iter().filter(|b| b.folder).count();
        bookmarks.retain(|b| {
            if b.folder {
                !b.children.is_empty()
            } else {
                true
            }
        });
        let after_count = bookmarks.iter().filter(|b| b.folder).count();

        removed_count += before_count - after_count;
        removed_count
    }

    /// Find all empty folders (for reporting)
    pub fn find_empty_folders(bookmarks: &[Bookmark], path: &str, results: &mut Vec<String>) {
        for bookmark in bookmarks {
            if bookmark.folder {
                let current_path = if path.is_empty() {
                    bookmark.title.clone()
                } else {
                    format!("{}/{}", path, bookmark.title)
                };

                if bookmark.children.is_empty() {
                    results.push(current_path.clone());
                } else {
                    Self::find_empty_folders(&bookmark.children, &current_path, results);
                }
            }
        }
    }
}

#[derive(Default)]
struct CleanupStats {
    duplicates_removed: usize,
    empty_folders_removed: usize,
}

/// Rule-based bookmark classification engine
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClassificationRule {
    /// Rule name/identifier
    pub name: String,
    /// Target folder name (Chinese)
    pub folder_name: String,
    /// Target folder name (English, for display)
    pub folder_name_en: String,
    /// URL patterns to match (case-insensitive)
    pub url_patterns: Vec<String>,
    /// Domain patterns to match
    pub domain_patterns: Vec<String>,
    /// Path patterns to match
    pub path_patterns: Vec<String>,
    /// Title patterns to match
    pub title_patterns: Vec<String>,
    /// Rule priority (higher = matched first)
    pub priority: i32,
    /// Rule description
    pub description: String,
    /// Rule specificity (auto-calculated: path=100, domain=50, title=10)
    #[serde(default)]
    pub specificity: i32,
}

/// Builder for `ClassificationRule`
#[derive(Default)]
pub struct ClassificationRuleBuilder {
    name: String,
    folder_name: String,
    folder_name_en: String,
    url_patterns: Vec<String>,
    domain_patterns: Vec<String>,
    path_patterns: Vec<String>,
    title_patterns: Vec<String>,
    priority: i32,
    description: String,
}

impl ClassificationRuleBuilder {
    pub fn new(name: &str, folder_name: &str, folder_name_en: &str) -> Self {
        Self {
            name: name.to_string(),
            folder_name: folder_name.to_string(),
            folder_name_en: folder_name_en.to_string(),
            ..Default::default()
        }
    }

    pub fn url_patterns(mut self, patterns: Vec<&str>) -> Self {
        self.url_patterns = patterns.iter().map(|s| s.to_lowercase()).collect();
        self
    }

    pub fn domain_patterns(mut self, patterns: Vec<&str>) -> Self {
        self.domain_patterns = patterns.iter().map(|s| s.to_lowercase()).collect();
        self
    }

    pub fn path_patterns(mut self, patterns: Vec<&str>) -> Self {
        self.path_patterns = patterns.iter().map(|s| s.to_lowercase()).collect();
        self
    }

    pub fn title_patterns(mut self, patterns: Vec<&str>) -> Self {
        self.title_patterns = patterns.iter().map(|s| s.to_lowercase()).collect();
        self
    }

    pub const fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub fn build(self) -> ClassificationRule {
        // 自动计算 specificity: 路径=100, 域名=50, 标题=10
        let specificity = Self::calculate_specificity(
            &self.path_patterns,
            &self.domain_patterns,
            &self.title_patterns,
        );
        ClassificationRule {
            name: self.name,
            folder_name: self.folder_name,
            folder_name_en: self.folder_name_en,
            url_patterns: self.url_patterns,
            domain_patterns: self.domain_patterns,
            path_patterns: self.path_patterns,
            title_patterns: self.title_patterns,
            priority: self.priority,
            description: self.description,
            specificity,
        }
    }

    /// 计算规则具体度: 路径匹配=100, 域名匹配=50, 标题匹配=10
    const fn calculate_specificity(
        path_patterns: &[String],
        domain_patterns: &[String],
        title_patterns: &[String],
    ) -> i32 {
        let mut score = 0;
        if !path_patterns.is_empty() {
            score += 100;
        }
        if !domain_patterns.is_empty() {
            score += 50;
        }
        if !title_patterns.is_empty() {
            score += 10;
        }
        score
    }
}

impl ClassificationRule {
    /// Create a new classification rule (internal constructor)
    ///
    /// This function has 9 parameters which exceeds clippy's default of 7.
    /// We allow this because:
    /// 1. This is a private constructor used only within this module
    /// 2. All parameters are required (no optional fields)
    /// 3. Parameters are logically grouped: identity (3), patterns (4), metadata (2)
    /// 4. Public API uses `ClassificationRuleBuilder` for better ergonomics
    /// 5. Refactoring 95 call sites would add complexity without benefit
    pub fn new(
        name: &str,
        folder_name: &str,
        folder_name_en: &str,
        url_patterns: Vec<&str>,
        domain_patterns: Vec<&str>,
        path_patterns: Vec<&str>,
        title_patterns: Vec<&str>,
        priority: i32,
        description: &str,
    ) -> Self {
        // 自动计算 specificity
        let specificity = Self::calculate_specificity_from_patterns(
            !path_patterns.is_empty(),
            !domain_patterns.is_empty(),
            !title_patterns.is_empty(),
        );
        Self {
            name: name.to_string(),
            folder_name: folder_name.to_string(),
            folder_name_en: folder_name_en.to_string(),
            url_patterns: url_patterns.iter().map(|s| s.to_lowercase()).collect(),
            domain_patterns: domain_patterns.iter().map(|s| s.to_lowercase()).collect(),
            path_patterns: path_patterns.iter().map(|s| s.to_lowercase()).collect(),
            title_patterns: title_patterns.iter().map(|s| s.to_lowercase()).collect(),
            priority,
            description: description.to_string(),
            specificity,
        }
    }

    /// Check if a bookmark matches this rule (improved accuracy)
    #[must_use]
    pub fn matches(&self, url: &str, title: &str) -> bool {
        let url_lower = url.to_lowercase();
        let title_lower = title.to_lowercase();

        // Extract domain and path from URL
        let (domain, path) = Self::parse_url_parts(&url_lower);

        // Check URL patterns (substring match for full URL patterns)
        for pattern in &self.url_patterns {
            if url_lower.contains(pattern) {
                return true;
            }
        }

        // Check domain patterns (EXACT match with wildcard support)
        for pattern in &self.domain_patterns {
            if Self::match_domain_exact(&domain, pattern) {
                return true;
            }
        }

        // Check path patterns (PREFIX match instead of contains)
        for pattern in &self.path_patterns {
            if path.starts_with(pattern) {
                return true;
            }
        }

        // Check title patterns (keyword match)
        for pattern in &self.title_patterns {
            if title_lower.contains(pattern) {
                return true;
            }
        }

        false
    }

    /// Check if a bookmark matches this rule and return the match reason
    fn matches_with_reason(&self, url: &str, title: &str) -> Option<String> {
        let url_lower = url.to_lowercase();
        let title_lower = title.to_lowercase();
        let (domain, path) = Self::parse_url_parts(&url_lower);

        // Check URL patterns
        for pattern in &self.url_patterns {
            if url_lower.contains(pattern) {
                return Some(format!("url_pattern: {pattern}"));
            }
        }

        // Check domain patterns
        for pattern in &self.domain_patterns {
            if Self::match_domain_exact(&domain, pattern) {
                return Some(format!("domain: {pattern}"));
            }
        }

        // Check path patterns
        for pattern in &self.path_patterns {
            if path.starts_with(pattern) {
                return Some(format!("path: {pattern}"));
            }
        }

        // Check title patterns
        for pattern in &self.title_patterns {
            if title_lower.contains(pattern) {
                return Some(format!("title: {pattern}"));
            }
        }

        None
    }

    /// Exact domain matching with wildcard support
    /// - "example.com" matches only "example.com"
    /// - "*.example.com" matches "sub.example.com" and "example.com"
    fn match_domain_exact(domain: &str, pattern: &str) -> bool {
        if let Some(suffix) = pattern.strip_prefix("*.") {
            // Wildcard match: *.example.com
            // Match sub.example.com or example.com itself
            domain == suffix || domain.ends_with(&format!(".{suffix}"))
        } else {
            // Exact match
            domain == pattern
        }
    }

    fn parse_url_parts(url: &str) -> (String, String) {
        let without_protocol = url
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        if let Some(slash_pos) = without_protocol.find('/') {
            let domain = without_protocol[..slash_pos].to_string();
            let path = without_protocol[slash_pos..].to_string();
            (domain, path)
        } else {
            (without_protocol.to_string(), String::new())
        }
    }

    /// 计算规则具体度: 路径匹配=100, 域名匹配=50, 标题匹配=10
    #[must_use]
    pub const fn calculate_specificity_from_patterns(
        has_path: bool,
        has_domain: bool,
        has_title: bool,
    ) -> i32 {
        let mut score = 0;
        if has_path {
            score += 100;
        }
        if has_domain {
            score += 50;
        }
        if has_title {
            score += 10;
        }
        score
    }
}

/// Built-in classification rules
pub fn get_builtin_rules() -> Vec<ClassificationRule> {
    const JSON: &str = include_str!("assets/builtin_rules.json");
    match serde_json::from_str(JSON) {
        Ok(rules) => rules,
        Err(e) => {
            tracing::error!(
                "解析内置分类规则 (builtin_rules.json) 严重失败: {}，将返回空规则集",
                e
            );
            Vec::new()
        }
    }
}

/// Classification statistics
#[derive(Default)]
struct ClassificationStats {
    total_processed: usize,
    total_classified: usize,
    by_category: HashMap<String, usize>,
    unclassified: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassifierBackend {
    Auto,
    Ollama,
    Rules,
}

impl ClassifierBackend {
    fn parse(value: &str) -> Result<Self> {
        match value.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "ollama" => Ok(Self::Ollama),
            "rules" => Ok(Self::Rules),
            other => anyhow::bail!("Unsupported classifier '{other}'. Use auto, ollama, or rules."),
        }
    }
}

struct ClassificationConfig<'a> {
    rules_file: Option<&'a str>,
    classifier: &'a str,
    ollama_model: &'a str,
    ollama_url: &'a str,
    ollama_batch_size: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
struct OllamaBookmarkInput {
    id: String,
    title: String,
    url: String,
    domain: String,
    path: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct OllamaBatchResponse {
    items: Vec<OllamaBookmarkOutput>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct OllamaBookmarkOutput {
    id: String,
    folder_name: String,
    #[serde(default)]
    folder_name_en: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[derive(Debug, serde::Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    format: &'a str,
}

impl SyncEngine {
    /// Print built-in classification rules
    pub fn print_builtin_rules() {
        let rules = get_builtin_rules();

        println!("\n🧠 Built-in Classification Rules");
        println!(
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"
        );

        for rule in &rules {
            println!("📁 {} / {}", rule.folder_name, rule.folder_name_en);
            println!("   Priority: {} | Rule: {}", rule.priority, rule.name);
            println!("   {}", rule.description);

            if !rule.domain_patterns.is_empty() {
                let domains: Vec<_> = rule.domain_patterns.iter().take(5).collect();
                let more = if rule.domain_patterns.len() > 5 {
                    format!(" (+{} more)", rule.domain_patterns.len() - 5)
                } else {
                    String::new()
                };
                println!(
                    "   Domains: {}{}",
                    domains
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    more
                );
            }

            if !rule.url_patterns.is_empty() {
                println!("   URL patterns: {}", rule.url_patterns.join(", "));
            }

            if !rule.path_patterns.is_empty() {
                let paths: Vec<_> = rule.path_patterns.iter().take(5).collect();
                println!(
                    "   Path patterns: {}",
                    paths
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            println!();
        }

        println!(
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        );
        println!("Total: {} rules\n", rules.len());
        println!("💡 Tip: Use --rules-file to load custom rules from a JSON file.");
        println!("   Example JSON format:");
        println!("   {{");
        println!("     \"name\": \"custom\",");
        println!("     \"folder_name\": \"自定义\",");
        println!("     \"folder_name_en\": \"Custom\",");
        println!("     \"url_patterns\": [\"pattern1\", \"pattern2\"],");
        println!("     \"domain_patterns\": [\"example.com\"],");
        println!("     \"path_patterns\": [\"/custom\"],");
        println!("     \"title_patterns\": [\"custom\"],");
        println!("     \"priority\": 100,");
        println!("     \"description\": \"Custom rule description\"");
        println!("   }}\n");
    }

    fn load_classification_rules(rules_file: Option<&str>) -> Result<Vec<ClassificationRule>> {
        let mut rules = get_builtin_rules();
        if let Some(file_path) = rules_file {
            info!("📂 Loading custom rules from: {}", file_path);
            let content =
                std::fs::read_to_string(file_path).context("Failed to read rules file")?;
            let custom_rules: Vec<ClassificationRule> =
                serde_json::from_str(&content).context("Failed to parse rules file")?;
            info!("✅ Loaded {} custom rules", custom_rules.len());
            rules.extend(custom_rules);
        }

        rules.sort_by(|a, b| match b.priority.cmp(&a.priority) {
            std::cmp::Ordering::Equal => b.specificity.cmp(&a.specificity),
            other => other,
        });
        Ok(rules)
    }

    fn parse_url_for_llm(url: &str) -> (String, String) {
        let without_protocol = url
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        if let Some(slash_pos) = without_protocol.find('/') {
            (
                without_protocol[..slash_pos].to_string(),
                without_protocol[slash_pos..].to_string(),
            )
        } else {
            (without_protocol.to_string(), "/".to_string())
        }
    }

    fn sanitize_folder_name(name: &str, fallback: &str) -> String {
        let sanitized = name.trim().replace(['/', '\\', ':', '\n', '\r'], " ");
        let compact = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.is_empty() {
            fallback.to_string()
        } else {
            compact.chars().take(24).collect()
        }
    }

    fn extract_json_payload(raw: &str) -> &str {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return trimmed;
        }
        if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                return &trimmed[start..=end];
            }
        }
        trimmed
    }

    async fn classify_with_rules(
        to_classify: &[Bookmark],
        rules: &[ClassificationRule],
        verbose: bool,
    ) -> (
        HashMap<String, Vec<Bookmark>>,
        Vec<Bookmark>,
        ClassificationStats,
    ) {
        let mut classified: HashMap<String, Vec<Bookmark>> = HashMap::new();
        let mut unclassified: Vec<Bookmark> = Vec::new();
        let mut stats = ClassificationStats {
            total_processed: to_classify.len(),
            ..Default::default()
        };

        for bookmark in to_classify {
            let url = bookmark.url.as_deref().unwrap_or("");
            let title = &bookmark.title;

            let mut matched = false;
            for rule in rules {
                if let Some(reason) = rule.matches_with_reason(url, title) {
                    if verbose {
                        info!(
                            "  ✓ '{}' -> {} (rule: {}, reason: {})",
                            title, rule.folder_name, rule.name, reason
                        );
                    }
                    classified
                        .entry(rule.folder_name.clone())
                        .or_default()
                        .push(bookmark.clone());
                    *stats
                        .by_category
                        .entry(rule.folder_name.clone())
                        .or_insert(0) += 1;
                    matched = true;
                    break;
                }
            }

            if !matched {
                unclassified.push(bookmark.clone());
                stats.unclassified += 1;
            }
        }

        stats.total_classified = stats.total_processed - stats.unclassified;
        (classified, unclassified, stats)
    }

    async fn classify_with_ollama(
        to_classify: &[Bookmark],
        model: &str,
        base_url: &str,
        batch_size: usize,
        verbose: bool,
    ) -> Result<(
        HashMap<String, Vec<Bookmark>>,
        Vec<Bookmark>,
        ClassificationStats,
    )> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .context("Failed to build Ollama HTTP client")?;

        let batch_size = batch_size.max(1);
        let mut classified: HashMap<String, Vec<Bookmark>> = HashMap::new();
        let mut unclassified = Vec::new();
        let mut stats = ClassificationStats {
            total_processed: to_classify.len(),
            ..Default::default()
        };
        let mut known_folders: HashMap<String, String> = HashMap::new();

        for batch in to_classify.chunks(batch_size) {
            let inputs: Vec<OllamaBookmarkInput> = batch
                .iter()
                .map(|bookmark| {
                    let url = if let Some(u) = bookmark.url.clone() {
                        u
                    } else {
                        tracing::debug!(
                            "书签 ID={} ('{}') 没有 URL 字段，作为无网址节点传递给模型",
                            bookmark.id,
                            bookmark.title
                        );
                        String::new()
                    };
                    let (domain, path) = Self::parse_url_for_llm(&url);
                    OllamaBookmarkInput {
                        id: bookmark.id.clone(),
                        title: bookmark.title.clone(),
                        url,
                        domain,
                        path,
                    }
                })
                .collect();

            let known_folder_list = if known_folders.is_empty() {
                "[]".to_string()
            } else {
                serde_json::to_string_pretty(&known_folders)?
            };
            let items_json = serde_json::to_string_pretty(&inputs)?;
            let prompt = format!(
                "你是浏览器书签分类器。请按语义给书签分配文件夹，而不是按硬编码规则。\n\
已知文件夹映射（优先复用，避免同义重复）：\n{known_folder_list}\n\n\
要求：\n\
1. 返回严格 JSON，对象格式必须是 {{\"items\":[...]}}。\n\
2. 每个 item 必须包含 id, folder_name, folder_name_en, confidence, reason。\n\
3. folder_name 使用简洁中文，2-8个字，适合作为文件夹名。\n\
4. folder_name_en 使用简洁英文，1-4个词。\n\
5. 若现有文件夹可复用，优先复用现有 folder_name。\n\
6. 若信息不足，归到“待复查” / \"Review\"。\n\
7. 仅依据 title、url、domain、path 推断，不要输出额外解释。\n\n\
待分类书签：\n{items_json}"
            );

            let req = OllamaGenerateRequest {
                model,
                prompt,
                stream: false,
                format: "json",
            };
            let endpoint = format!("{}/api/generate", base_url.trim_end_matches('/'));
            let response = client
                .post(endpoint)
                .json(&req)
                .send()
                .await
                .context("Failed to call Ollama API")?
                .error_for_status()
                .context("Ollama API returned error status")?;
            let payload: OllamaGenerateResponse = response
                .json()
                .await
                .context("Failed to parse Ollama response")?;
            let json_payload = Self::extract_json_payload(&payload.response);
            let parsed: OllamaBatchResponse =
                serde_json::from_str(json_payload).with_context(|| {
                    format!("Failed to parse Ollama JSON payload: {}", payload.response)
                })?;
            let assignment_map: HashMap<_, _> = parsed
                .items
                .into_iter()
                .map(|item| (item.id.clone(), item))
                .collect();

            for bookmark in batch {
                let Some(output) = assignment_map.get(&bookmark.id) else {
                    unclassified.push(bookmark.clone());
                    stats.unclassified += 1;
                    continue;
                };

                let folder_name = Self::sanitize_folder_name(&output.folder_name, "待复查");
                let folder_name_en = Self::sanitize_folder_name(
                    output.folder_name_en.as_deref().unwrap_or("Review"),
                    "Review",
                );
                let reason = output
                    .reason
                    .clone()
                    .unwrap_or_else(|| "ollama".to_string());
                let confidence = output.confidence.unwrap_or(0.0);

                known_folders
                    .entry(folder_name.clone())
                    .or_insert(folder_name_en.clone());

                if folder_name == "待复查" || confidence < 0.35 {
                    if verbose {
                        info!(
                            "  ? '{}' -> 待复查 (confidence: {:.2}, reason: {})",
                            bookmark.title, confidence, reason
                        );
                    }
                    unclassified.push(bookmark.clone());
                    stats.unclassified += 1;
                    continue;
                }

                if verbose {
                    info!(
                        "  ✓ '{}' -> {} (ollama, confidence: {:.2}, reason: {})",
                        bookmark.title, folder_name, confidence, reason
                    );
                }
                classified
                    .entry(folder_name.clone())
                    .or_default()
                    .push(bookmark.clone());
                *stats.by_category.entry(folder_name).or_insert(0) += 1;
            }
        }

        stats.total_classified = stats.total_processed - stats.unclassified;
        Ok((classified, unclassified, stats))
    }

    async fn classify_bookmarks(
        to_classify: &[Bookmark],
        config: &ClassificationConfig<'_>,
        verbose: bool,
    ) -> Result<(
        HashMap<String, Vec<Bookmark>>,
        Vec<Bookmark>,
        ClassificationStats,
        String,
    )> {
        let backend = ClassifierBackend::parse(config.classifier)?;
        let rules = Self::load_classification_rules(config.rules_file)?;

        match backend {
            ClassifierBackend::Rules => {
                let (classified, unclassified, stats) =
                    Self::classify_with_rules(to_classify, &rules, verbose).await;
                Ok((classified, unclassified, stats, "rules".to_string()))
            }
            ClassifierBackend::Ollama => {
                let (classified, unclassified, stats) = Self::classify_with_ollama(
                    to_classify,
                    config.ollama_model,
                    config.ollama_url,
                    config.ollama_batch_size,
                    verbose,
                )
                .await?;
                Ok((classified, unclassified, stats, "ollama".to_string()))
            }
            ClassifierBackend::Auto => {
                match Self::classify_with_ollama(
                    to_classify,
                    config.ollama_model,
                    config.ollama_url,
                    config.ollama_batch_size,
                    verbose,
                )
                .await
                {
                    Ok((classified, unclassified, stats)) => {
                        info!("🧠 Classifier backend: Ollama ({})", config.ollama_model);
                        Ok((classified, unclassified, stats, "ollama".to_string()))
                    }
                    Err(e) => {
                        warn!(
                            "Ollama classification unavailable, falling back to rules: {}",
                            e
                        );
                        let (classified, unclassified, stats) =
                            Self::classify_with_rules(to_classify, &rules, verbose).await;
                        Ok((classified, unclassified, stats, "rules".to_string()))
                    }
                }
            }
        }
    }

    fn apply_classification_results(
        bookmarks: &mut Vec<Bookmark>,
        classified: &HashMap<String, Vec<Bookmark>>,
        unclassified: &[Bookmark],
    ) {
        for (folder_name, items) in classified {
            let existing_folder = bookmarks
                .iter_mut()
                .find(|b| b.folder && b.title == *folder_name);

            if let Some(folder) = existing_folder {
                folder.children.extend(items.clone());
            } else {
                let new_folder = Bookmark {
                    id: format!("smart-folder-{}", chrono::Utc::now().timestamp_millis()),
                    title: folder_name.clone(),
                    url: None,
                    folder: true,
                    children: items.clone(),
                    date_added: Some(chrono::Utc::now().timestamp_millis()),
                    date_modified: Some(chrono::Utc::now().timestamp_millis()),
                };
                bookmarks.push(new_folder);
            }
        }

        if !unclassified.is_empty() {
            let unclassified_folder = bookmarks
                .iter_mut()
                .find(|b| b.folder && b.title == "未分类");

            if let Some(folder) = unclassified_folder {
                folder.children.extend(unclassified.to_vec());
            } else {
                let new_folder = Bookmark {
                    id: format!(
                        "unclassified-folder-{}",
                        chrono::Utc::now().timestamp_millis()
                    ),
                    title: "未分类".to_string(),
                    url: None,
                    folder: true,
                    children: unclassified.to_vec(),
                    date_added: Some(chrono::Utc::now().timestamp_millis()),
                    date_modified: Some(chrono::Utc::now().timestamp_millis()),
                };
                bookmarks.push(new_folder);
            }
        }
    }

    /// Smart organize bookmarks using rule engine
    pub async fn smart_organize(
        &mut self,
        browser_names: Option<&str>,
        rules_file: Option<&str>,
        classifier: &str,
        ollama_model: &str,
        ollama_url: &str,
        ollama_batch_size: usize,
        uncategorized_only: bool,
        show_stats: bool,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
        info!("🧠 Starting smart bookmark organization");
        let classify_config = ClassificationConfig {
            rules_file,
            classifier,
            ollama_model,
            ollama_url,
            ollama_batch_size,
        };

        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_lowercase().replace('-', " "))
                .collect();

            self.adapters
                .iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    let name_normalized = name.replace('-', " ");
                    browser_list.iter().any(|b| {
                        // 精确匹配或完整词匹配
                        if name == *b || name_normalized == *b {
                            return true;
                        }
                        // "brave" 不应该匹配 "brave nightly"
                        if *b == "brave" && name_normalized.contains("nightly") {
                            return false;
                        }
                        // 部分匹配
                        name.contains(b) || name_normalized.contains(b)
                    })
                })
                .collect()
        } else {
            self.adapters.iter().collect()
        };

        if target_adapters.is_empty() {
            anyhow::bail!("No browsers found for organization");
        }

        info!("🎯 Target browsers:");
        for adapter in &target_adapters {
            info!("  - {}", adapter.browser_type().name());
        }

        // Process each browser
        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();

            match Self::read_single_profile_for_write(adapter.as_ref(), "organize bookmarks in") {
                Ok((mut bookmarks, version)) => {
                    info!("\n📊 {} : Processing...", browser_name);

                    // Collect bookmarks to classify
                    let mut to_classify: Vec<Bookmark> = Vec::new();
                    if uncategorized_only {
                        // Only collect bookmarks at root level (not in folders)
                        Self::collect_root_bookmarks(&mut bookmarks, &mut to_classify);
                    } else {
                        // Collect all non-folder bookmarks from entire tree
                        Self::collect_all_bookmarks_for_classification(
                            &mut bookmarks,
                            &mut to_classify,
                        );
                    }

                    info!("  📖 Found {} bookmarks to classify", to_classify.len());
                    let (classified, unclassified, stats, backend_used) =
                        Self::classify_bookmarks(&to_classify, &classify_config, verbose).await?;
                    info!("  🧠 Backend: {}", backend_used);
                    Self::apply_classification_results(&mut bookmarks, &classified, &unclassified);

                    for (folder_name, items) in &classified {
                        info!("  📁 {} : {} bookmarks", folder_name, items.len());
                    }
                    if !unclassified.is_empty() {
                        info!("  📁 未分类 : {} bookmarks", unclassified.len());
                    }

                    // Show statistics if requested
                    if show_stats {
                        println!("\n📊 Classification Statistics for {browser_name}:");
                        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                        println!("  Total processed:  {}", stats.total_processed);
                        println!(
                            "  Total classified: {} ({:.1}%)",
                            stats.total_classified,
                            if stats.total_processed > 0 {
                                stats.total_classified as f64 / stats.total_processed as f64 * 100.0
                            } else {
                                0.0
                            }
                        );
                        println!("  Unclassified:     {}", stats.unclassified);
                        println!("\n  By category:");

                        let mut categories: Vec<_> = stats.by_category.iter().collect();
                        categories.sort_by(|a, b| b.1.cmp(a.1));
                        for (category, count) in categories {
                            println!("    📁 {category} : {count}");
                        }
                        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                    }

                    if dry_run {
                        info!(
                            "  🏃 Dry run - would classify {} bookmarks into {} folders",
                            stats.total_classified,
                            classified.len()
                        );
                    } else if stats.total_classified > 0 {
                        let backup_path = adapter.backup_bookmarks().with_context(|| {
                            format!("Refusing to organize {browser_name}: backup failed")
                        })?;
                        info!("  💾 Backup created: {:?}", backup_path);

                        // Write organized bookmarks
                        let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
                        adapter
                            .write_bookmarks(&bookmarks)
                            .context("Failed to write organized bookmarks")?;
                        info!("  ✅ Organization complete");
                    } else {
                        info!("  ✨ No bookmarks to classify");
                    }
                }
                Err(e) => return Err(e),
            }
        }

        info!("\n✅ Smart organization complete!");
        Ok(())
    }

    /// Collect bookmarks at root level only (not in folders)
    fn collect_root_bookmarks(bookmarks: &mut Vec<Bookmark>, collected: &mut Vec<Bookmark>) {
        let mut indices_to_remove = Vec::new();

        for (i, bookmark) in bookmarks.iter().enumerate() {
            if !bookmark.folder {
                collected.push(bookmark.clone());
                indices_to_remove.push(i);
            }
        }

        for &i in indices_to_remove.iter().rev() {
            bookmarks.remove(i);
        }
    }

    /// Collect all bookmarks from entire tree for classification
    pub(crate) fn collect_all_bookmarks_for_classification(
        bookmarks: &mut Vec<Bookmark>,
        collected: &mut Vec<Bookmark>,
    ) {
        // 只保护已分类的文件夹，不保护系统文件夹（需要递归进入）
        let classified_folders = [
            // 分类文件夹 (48个规则对应的文件夹) - 注意：不包含"未分类"
            "登录入口",
            "社交媒体",
            "视频流媒体",
            "开发工具",
            "购物网站",
            "新闻资讯",
            "文档参考",
            "云存储",
            "邮箱通讯",
            "金融理财",
            "AI工具",
            "设计创意",
            "教育学习",
            "音乐音频",
            "游戏娱乐",
            "论坛社区",
            "管理后台",
            "API服务",
            "应用商店",
            "存档资料",
            "百科知识",
            "文件托管",
            "搜索引擎",
            "NSFW内容",
            "Discord社群",
            "动漫二次元",
            "下载资源",
            "安全隐私",
            "Linux开源",
            "微软服务",
            "苹果服务",
            "谷歌服务",
            "联邦宇宙",
            "移动开发",
            "科学研究",
            "直播平台",
            "浏览器扩展",
            "在线工具",
            "效率工具",
            "游戏社区",
            "图床托管",
            "导航目录",
            "中文平台",
            "设计素材",
            "硬件技术",
            "托管项目",
            "博客站点",
            "开发者工具",
            "网站主页", // 注意："未分类"不在此列表中，允许重新分类
        ];

        // 跳过的系统文件夹（不收集其中的书签，但也不递归）
        let skip_folders = ["com.apple.ReadingList", "阅读列表", "History"];

        // 临时文件夹名称（需要抽离整理的）
        let temp_folders = ["👀临时", "👀 临时", "临时", "temp", "temporary"];

        // First pass: recursively process children
        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder {
                let title_lower = bookmark.title.to_lowercase();
                let is_temp_folder = temp_folders.iter().any(|tf| {
                    title_lower == tf.to_lowercase() || title_lower.contains(&tf.to_lowercase())
                });

                if is_temp_folder {
                    // 特殊处理：收集临时文件夹中的所有书签进行分类整理
                    info!("  📦 从临时文件夹 '{}' 抽取书签进行整理", bookmark.title);
                    Self::collect_from_temp_folder(&mut bookmark.children, collected);
                } else if bookmark.title == "未分类" {
                    // 特殊处理：收集"未分类"文件夹中的所有书签进行重新分类
                    Self::collect_from_unclassified(&mut bookmark.children, collected);
                } else if skip_folders.contains(&bookmark.title.as_str()) {
                    // 跳过阅读列表等系统文件夹
                    continue;
                } else if classified_folders.contains(&bookmark.title.as_str()) {
                    // 已分类文件夹：不重新分类，保持原样
                    continue;
                } else {
                    // 其他所有文件夹（包括BookmarksBar、导入的浏览器书签等）：递归处理
                    Self::collect_all_bookmarks_for_classification(
                        &mut bookmark.children,
                        collected,
                    );
                }
            }
        }

        // Second pass: collect non-folder bookmarks at current level
        let mut indices_to_remove = Vec::new();
        for (i, bookmark) in bookmarks.iter().enumerate() {
            if !bookmark.folder {
                collected.push(bookmark.clone());
                indices_to_remove.push(i);
            }
        }

        for &i in indices_to_remove.iter().rev() {
            bookmarks.remove(i);
        }
    }

    /// Collect all bookmarks from "未分类" folder for re-classification
    fn collect_from_unclassified(bookmarks: &mut Vec<Bookmark>, collected: &mut Vec<Bookmark>) {
        let mut indices_to_remove = Vec::new();

        for (i, bookmark) in bookmarks.iter().enumerate() {
            if !bookmark.folder {
                collected.push(bookmark.clone());
                indices_to_remove.push(i);
            }
        }

        // Remove collected bookmarks from "未分类" folder
        for &i in indices_to_remove.iter().rev() {
            bookmarks.remove(i);
        }
    }

    /// Collect all bookmarks from temp folder (👀临时) for organization
    /// 从临时文件夹抽取书签进行整理，分类后从临时文件夹移除
    fn collect_from_temp_folder(bookmarks: &mut Vec<Bookmark>, collected: &mut Vec<Bookmark>) {
        let mut indices_to_remove = Vec::new();

        for (i, bookmark) in bookmarks.iter().enumerate() {
            if bookmark.folder {
                // 递归处理子文件夹
                let mut sub_collected = Vec::new();
                Self::collect_from_temp_folder_recursive(&bookmark.children, &mut sub_collected);
                collected.extend(sub_collected);
                // 标记整个子文件夹待移除（如果里面的书签都被抽走了）
                indices_to_remove.push(i);
            } else {
                collected.push(bookmark.clone());
                indices_to_remove.push(i);
            }
        }

        // Remove collected items from temp folder
        for &i in indices_to_remove.iter().rev() {
            bookmarks.remove(i);
        }
    }

    /// Recursively collect bookmarks from temp folder (read-only, for nested folders)
    fn collect_from_temp_folder_recursive(bookmarks: &[Bookmark], collected: &mut Vec<Bookmark>) {
        for bookmark in bookmarks {
            if bookmark.folder {
                Self::collect_from_temp_folder_recursive(&bookmark.children, collected);
            } else {
                collected.push(bookmark.clone());
            }
        }
    }

    /// Smart organize bookmarks from an exported file (HTML/JSON)
    pub async fn smart_organize_file(
        &mut self,
        input_file: &str,
        output_file: &str,
        rules_file: Option<&str>,
        classifier: &str,
        ollama_model: &str,
        ollama_url: &str,
        ollama_batch_size: usize,
        show_stats: bool,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
        info!("🧠 Organizing bookmarks from file: {}", input_file);
        let classify_config = ClassificationConfig {
            rules_file,
            classifier,
            ollama_model,
            ollama_url,
            ollama_batch_size,
        };

        // Read input file
        let content = std::fs::read_to_string(input_file)
            .context(format!("Failed to read input file: {input_file}"))?;

        // Parse bookmarks based on file type
        let mut bookmarks = if input_file.ends_with(".json") {
            serde_json::from_str::<Vec<Bookmark>>(&content)
                .context("Failed to parse JSON bookmark file")?
        } else if input_file.ends_with(".html") || input_file.ends_with(".htm") {
            parse_safari_html(&content)?
        } else {
            // Try JSON first, then HTML
            serde_json::from_str::<Vec<Bookmark>>(&content)
                .or_else(|_| parse_safari_html(&content))
                .context("Failed to parse bookmark file (tried JSON and HTML)")?
        };

        info!(
            "📖 Loaded {} bookmarks from file",
            Self::count_all_bookmarks(&bookmarks)
        );

        // Flatten: remove browser root folders (Brave, Brave Nightly, Waterfox, etc.)
        info!("📦 Flattening: removing browser root folders...");
        let flat_config = FlatExportConfig {
            flatten_root: true,
            root_folders_to_remove: None,
        };
        let (flattened, flatten_stats) = flatten_bookmarks(&bookmarks, &flat_config);
        bookmarks = flattened;
        if flatten_stats.root_folders_removed > 0 {
            info!(
                "  ✅ Removed {} browser root folders, promoted {} items",
                flatten_stats.root_folders_removed, flatten_stats.bookmarks_promoted
            );
        }

        // Collect all bookmarks for classification
        let mut to_classify: Vec<Bookmark> = Vec::new();
        Self::collect_all_bookmarks_for_classification(&mut bookmarks, &mut to_classify);

        info!("  📖 Found {} bookmarks to classify", to_classify.len());
        let (classified, unclassified, stats, backend_used) =
            Self::classify_bookmarks(&to_classify, &classify_config, verbose).await?;
        info!("  🧠 Backend: {}", backend_used);
        Self::apply_classification_results(&mut bookmarks, &classified, &unclassified);

        for (folder_name, items) in &classified {
            info!("  📁 {} : {} bookmarks", folder_name, items.len());
        }
        if !unclassified.is_empty() {
            info!("  📁 未分类 : {} bookmarks", unclassified.len());
        }

        // Show statistics
        if show_stats {
            println!("\n📊 Classification Statistics:");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  Total processed:  {}", stats.total_processed);
            println!(
                "  Total classified: {} ({:.1}%)",
                stats.total_classified,
                if stats.total_processed > 0 {
                    stats.total_classified as f64 / stats.total_processed as f64 * 100.0
                } else {
                    0.0
                }
            );
            println!("  Unclassified:     {}", stats.unclassified);
            println!("\n  By category:");

            let mut categories: Vec<_> = stats.by_category.iter().collect();
            categories.sort_by(|a, b| b.1.cmp(a.1));
            for (category, count) in categories {
                println!("    📁 {category} : {count}");
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        }

        if dry_run {
            info!(
                "  🏃 Dry run - would classify {} bookmarks into {} folders",
                stats.total_classified,
                classified.len()
            );
        } else {
            // Wrap all bookmarks in 📁镜像文件夹 + merge temp folders
            info!("📦 Wrapping all bookmarks in root folder: \"📁镜像文件夹\"");

            // Extract and MERGE all protected folders (👀临时) into one
            let mut merged_temp_children: Vec<Bookmark> = Vec::new();
            bookmarks.retain(|b| {
                if b.folder
                    && PROTECTED_FOLDERS
                        .iter()
                        .any(|p| b.title.to_lowercase().contains(&p.to_lowercase()))
                {
                    merged_temp_children.extend(b.children.clone());
                    false
                } else {
                    true
                }
            });

            let wrapped = Bookmark {
                id: "wrap-mirror-folder".to_string(),
                title: "📁镜像文件夹".to_string(),
                url: None,
                folder: true,
                children: bookmarks,
                date_added: Some(chrono::Utc::now().timestamp_millis()),
                date_modified: None,
            };

            let temp_folder = Bookmark {
                id: "temp-folder".to_string(),
                title: "👀临时".to_string(),
                url: None,
                folder: true,
                children: merged_temp_children,
                date_added: Some(chrono::Utc::now().timestamp_millis()),
                date_modified: None,
            };

            bookmarks = vec![wrapped, temp_folder];
            info!("  ✅ Merged all temp folders into single 👀临时");

            // Write output file
            let output_content = if output_file.ends_with(".json") {
                serde_json::to_string_pretty(&bookmarks)?
            } else {
                // Export as HTML
                Self::export_bookmarks_to_html_internal(&bookmarks)
            };

            crate::browser_utils::atomic_write_file(
                Path::new(output_file),
                output_content.as_bytes(),
            )
            .context(format!("Failed to write output file: {output_file}"))?;

            info!("  ✅ Organized bookmarks saved to: {}", output_file);
        }

        Ok(())
    }

    /// Export bookmarks to HTML format (Netscape Bookmark File Format)
    pub(crate) fn export_bookmarks_to_html_internal(bookmarks: &[Bookmark]) -> String {
        let mut html = String::from(
            "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n\
             <META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n\
             <TITLE>Bookmarks</TITLE>\n\
             <H1>Bookmarks</H1>\n\
             <DL><p>\n",
        );

        fn write_bookmark(html: &mut String, bookmark: &Bookmark, indent: usize) {
            let indent_str = "    ".repeat(indent);
            if bookmark.folder {
                html.push_str(&format!(
                    "{}<DT><H3>{}</H3>\n",
                    indent_str,
                    html_escape(&bookmark.title)
                ));
                html.push_str(&format!("{indent_str}<DL><p>\n"));
                for child in &bookmark.children {
                    write_bookmark(html, child, indent + 1);
                }
                html.push_str(&format!("{indent_str}</DL><p>\n"));
            } else if let Some(url) = &bookmark.url {
                html.push_str(&format!(
                    "{}<DT><A HREF=\"{}\">{}</A>\n",
                    indent_str,
                    html_escape(url),
                    html_escape(&bookmark.title)
                ));
            }
        }

        fn html_escape(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        }

        for bookmark in bookmarks {
            write_bookmark(&mut html, bookmark, 1);
        }

        html.push_str("</DL><p>\n");
        html
    }
}

fn parse_safari_html(html: &str) -> Result<Vec<Bookmark>> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let link_selector =
        Selector::parse("a").map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;

    let mut bookmarks = Vec::new();
    let mut id_counter = 1;

    for element in document.select(&link_selector) {
        if let Some(url) = element.value().attr("href") {
            let title = element.text().collect::<String>();

            bookmarks.push(Bookmark {
                id: format!("imported-{id_counter}"),
                title: title.trim().to_string(),
                url: Some(url.to_string()),
                folder: false,
                children: vec![],
                date_added: Some(chrono::Utc::now().timestamp_millis()),
                date_modified: Some(chrono::Utc::now().timestamp_millis()),
            });

            id_counter += 1;
        }
    }

    Ok(bookmarks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_bookmark(id: &str, title: &str, url: Option<&str>) -> Bookmark {
        Bookmark {
            id: id.to_string(),
            title: title.to_string(),
            url: url.map(ToString::to_string),
            folder: false,
            children: vec![],
            date_added: Some(1700000000000),
            date_modified: Some(1700000000000),
        }
    }

    fn create_folder(id: &str, title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: id.to_string(),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: Some(1700000000000),
            date_modified: Some(1700000000000),
        }
    }

    #[test]
    fn onion_exclusion_is_recursive_and_hostname_specific() {
        assert!(ExportConfig::default().exclude_onion);
        let mut bookmarks = vec![create_folder(
            "folder",
            "Nested",
            vec![
                create_bookmark("onion", "Onion", Some("HTTP://hidden.example.ONION./post")),
                create_bookmark(
                    "path",
                    "Path text only",
                    Some("https://example.com/path/.onion"),
                ),
                create_bookmark(
                    "suffix",
                    "Not an onion host",
                    Some("https://hidden.onion.example.com"),
                ),
            ],
        )];

        assert_eq!(remove_onion_bookmarks(&mut bookmarks), 1);
        assert_eq!(bookmarks[0].children.len(), 2);
        assert!(SyncEngine::requests_all_browsers(Some("all")));
        assert!(!SyncEngine::requests_all_browsers(Some("chrome")));
    }

    struct RestoreTestAdapter {
        path: PathBuf,
    }

    struct MultiProfileWriteTestAdapter {
        path: PathBuf,
    }

    impl BrowserAdapter for MultiProfileWriteTestAdapter {
        fn browser_type(&self) -> BrowserType {
            BrowserType::Chrome
        }

        fn detect_bookmark_path(&self) -> Result<PathBuf> {
            Ok(self.path.clone())
        }

        fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
            Ok(Vec::new())
        }

        fn read_all_bookmarks_with_profiles(&self) -> Result<Vec<(String, Vec<Bookmark>)>> {
            Ok(vec![
                ("Default".into(), Vec::new()),
                ("Work".into(), Vec::new()),
            ])
        }

        fn write_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<()> {
            panic!("multi-profile cleanup must not write")
        }

        fn backup_bookmarks(&self) -> Result<PathBuf> {
            panic!("multi-profile cleanup must refuse before backup")
        }

        fn validate_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<bool> {
            Ok(true)
        }
    }

    impl BrowserAdapter for RestoreTestAdapter {
        fn browser_type(&self) -> BrowserType {
            BrowserType::Chrome
        }

        fn detect_bookmark_path(&self) -> Result<PathBuf> {
            Ok(self.path.clone())
        }

        fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
            Ok(Vec::new())
        }

        fn write_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<()> {
            Ok(())
        }

        fn backup_bookmarks(&self) -> Result<PathBuf> {
            let backup = self.path.with_file_name(format!(
                "{}.restore-test-backup",
                self.path.file_name().unwrap().to_string_lossy()
            ));
            std::fs::copy(&self.path, &backup)?;
            Ok(backup)
        }

        fn validate_bookmarks(&self, _bookmarks: &[Bookmark]) -> Result<bool> {
            Ok(true)
        }
    }

    fn test_inventory(
        discovered_profiles: usize,
        harvested_profiles: usize,
        bookmark_count: usize,
        warnings: Vec<String>,
    ) -> InventoryReport {
        let installation = crate::data_types::manifest::BrowserInstallationSummary {
            installation: crate::data_types::models::BrowserInstallation {
                name: "Test Chrome".into(),
                kind: crate::data_types::models::BrowserKind::Chromium("Chrome".into()),
                channel: None,
                root_path: PathBuf::from("/tmp/test-chrome"),
            },
            profiles: (0..harvested_profiles)
                .map(|index| crate::data_types::manifest::BrowserProfileSummary {
                    profile: crate::data_types::models::BrowserProfile {
                        id: format!("profile-{index}"),
                        name: format!("Profile {index}"),
                        path: PathBuf::from(format!("/tmp/test-chrome/profile-{index}")),
                    },
                    bookmark_count,
                    folder_count: 0,
                    snapshot_path: None,
                    read_success: true,
                    error: None,
                })
                .collect(),
            discovery_error: None,
        };
        InventoryReport {
            schema_version: "1.0.0".into(),
            generated_at: chrono::Utc::now(),
            total_installations: 1,
            total_profiles: discovered_profiles,
            total_bookmarks: bookmark_count,
            warnings,
            installations: vec![installation],
        }
    }

    #[tokio::test]
    async fn restore_backup_finds_latest_matching_json_and_sqlite_backups() {
        for file_name in ["Bookmarks", "places.sqlite"] {
            let temp = tempfile::tempdir().unwrap();
            let current = temp.path().join(file_name);
            if file_name == "places.sqlite" {
                let db = rusqlite::Connection::open(&current).unwrap();
                db.execute("CREATE TABLE state (value TEXT NOT NULL)", [])
                    .unwrap();
                db.execute("INSERT INTO state VALUES ('current')", [])
                    .unwrap();
            } else {
                std::fs::write(&current, b"current").unwrap();
            }

            let legacy = if file_name == "Bookmarks" {
                temp.path().join("Bookmarks.json.backup")
            } else {
                temp.path().join("places.sqlite.backup")
            };
            if file_name == "places.sqlite" {
                let db = rusqlite::Connection::open(&legacy).unwrap();
                db.execute("CREATE TABLE state (value TEXT NOT NULL)", [])
                    .unwrap();
                db.execute("INSERT INTO state VALUES ('legacy')", [])
                    .unwrap();
            } else {
                std::fs::write(legacy, b"legacy").unwrap();
            }
            std::fs::write(
                temp.path().join(format!("{file_name}.backup-unrelated")),
                b"unrelated",
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
            let latest = temp.path().join(format!("{file_name}.backup-latest"));
            if file_name == "places.sqlite" {
                let db = rusqlite::Connection::open(&latest).unwrap();
                db.execute("CREATE TABLE state (value TEXT NOT NULL)", [])
                    .unwrap();
                db.execute("INSERT INTO state VALUES ('latest')", [])
                    .unwrap();
            } else {
                std::fs::write(&latest, b"latest").unwrap();
            }

            let mut engine = SyncEngine {
                adapters: vec![Box::new(RestoreTestAdapter {
                    path: current.clone(),
                })],
                last_sync_time: None,
            };
            engine.restore_backup("Chrome", None).await.unwrap();
            if file_name == "places.sqlite" {
                let db = rusqlite::Connection::open(current).unwrap();
                let value: String = db
                    .query_row("SELECT value FROM state", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(value, "latest");
            } else {
                assert_eq!(std::fs::read(current).unwrap(), b"latest");
            }
        }
    }

    #[tokio::test]
    async fn cleanup_refuses_an_ambiguous_multi_profile_write() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Bookmarks");
        std::fs::write(&path, b"{}").unwrap();
        let mut engine = SyncEngine {
            adapters: vec![Box::new(MultiProfileWriteTestAdapter { path })],
            last_sync_time: None,
        };

        let error = engine
            .cleanup_bookmarks(Some("chrome"), true, true, false, false)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("2 writable profiles"));
        assert!(error
            .to_string()
            .contains("cannot target one profile exactly"));
    }

    #[test]
    fn master_backup_rejects_empty_or_incomplete_inventory() {
        assert!(validate_master_backup_inventory(&test_inventory(0, 0, 0, vec![])).is_err());
        assert!(validate_master_backup_inventory(&test_inventory(1, 0, 1, vec![])).is_err());
        assert!(validate_master_backup_inventory(&test_inventory(
            1,
            1,
            1,
            vec!["profile warning".into()]
        ))
        .is_err());
        assert!(validate_master_backup_inventory(&test_inventory(1, 1, 1, vec![])).is_ok());
    }

    #[test]
    fn html_export_replaces_existing_output() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("bookmarks.html");
        std::fs::write(&output, b"stale").unwrap();

        export_bookmarks_to_html(
            &[create_bookmark("1", "Example", Some("https://example.com"))],
            output.to_str().unwrap(),
        )
        .unwrap();

        let content = std::fs::read_to_string(output).unwrap();
        assert!(content.contains("https://example.com"));
        assert!(!content.contains("stale"));
    }

    #[test]
    fn test_normalize_url() {
        assert_eq!(
            SyncEngine::normalize_url("https://example.com/"),
            "https://example.com/"
        );
        assert_eq!(
            SyncEngine::normalize_url("https://example.com"),
            "https://example.com"
        );
        assert_eq!(
            SyncEngine::normalize_url("HTTPS://EXAMPLE.COM/"),
            "HTTPS://EXAMPLE.COM/"
        );
        assert_eq!(
            SyncEngine::normalize_url("https://example.com#section"),
            "https://example.com#section"
        );
        assert_eq!(
            SyncEngine::normalize_url("  https://example.com/  "),
            "  https://example.com/  "
        );
        assert_ne!(
            SyncEngine::normalize_url("https://example.com/A"),
            SyncEngine::normalize_url("https://example.com/a")
        );
    }

    #[test]
    fn test_count_all_bookmarks() {
        let bookmarks = vec![
            create_bookmark("1", "Test1", Some("https://test1.com")),
            create_folder(
                "2",
                "Folder1",
                vec![
                    create_bookmark("3", "Test2", Some("https://test2.com")),
                    create_bookmark("4", "Test3", Some("https://test3.com")),
                ],
            ),
        ];
        assert_eq!(SyncEngine::count_all_bookmarks(&bookmarks), 3);
    }

    #[test]
    fn test_count_all_folders() {
        let bookmarks = vec![
            create_bookmark("1", "Test1", Some("https://test1.com")),
            create_folder(
                "2",
                "Folder1",
                vec![create_folder(
                    "3",
                    "SubFolder",
                    vec![create_bookmark("4", "Test2", Some("https://test2.com"))],
                )],
            ),
        ];
        assert_eq!(SyncEngine::count_all_folders(&bookmarks), 2);
    }

    #[test]
    fn test_is_homepage_url() {
        assert!(SyncEngine::is_homepage_url("https://example.com"));
        assert!(SyncEngine::is_homepage_url("https://example.com/"));
        assert!(SyncEngine::is_homepage_url("http://example.com"));
        assert!(SyncEngine::is_homepage_url("https://sub.example.com"));

        assert!(!SyncEngine::is_homepage_url("https://example.com/path"));
        assert!(!SyncEngine::is_homepage_url("https://example.com/path/"));
        assert!(!SyncEngine::is_homepage_url("ftp://example.com"));
    }

    #[test]
    fn test_remove_empty_folders() {
        let mut bookmarks = vec![
            create_folder("1", "EmptyFolder", vec![]),
            create_folder(
                "2",
                "NonEmptyFolder",
                vec![create_bookmark("3", "Test", Some("https://test.com"))],
            ),
            create_folder(
                "4",
                "NestedEmpty",
                vec![create_folder("5", "InnerEmpty", vec![])],
            ),
        ];

        let removed = SyncEngine::remove_empty_folders(&mut bookmarks);
        assert_eq!(removed, 3); // EmptyFolder, NestedEmpty, and InnerEmpty
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].title, "NonEmptyFolder");
    }

    #[test]
    fn test_deduplicate_bookmarks_global() {
        let mut bookmarks = vec![
            create_bookmark("1", "Dup1", Some("https://example.com")),
            create_folder(
                "2",
                "Folder",
                vec![
                    create_bookmark("3", "Dup2", Some("https://example.com")), // duplicate - deeper, should keep
                ],
            ),
            create_bookmark("4", "Other", Some("https://other.com")),
        ];

        SyncEngine::deduplicate_bookmarks_global(&mut bookmarks);

        let total = SyncEngine::count_all_bookmarks(&bookmarks);
        assert_eq!(total, 2); // One example.com and one other.com
    }

    #[test]
    fn test_find_folder_by_path() {
        let bookmarks = vec![create_folder(
            "1",
            "Work",
            vec![create_folder(
                "2",
                "Projects",
                vec![create_bookmark(
                    "3",
                    "Project1",
                    Some("https://project1.com"),
                )],
            )],
        )];

        let found = SyncEngine::find_folder_by_path(&bookmarks, "Work/Projects");
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Projects");

        let not_found = SyncEngine::find_folder_by_path(&bookmarks, "Work/NonExistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_parse_safari_html() {
        let html = r#"
            <html>
            <body>
                <a href="https://example.com">Example</a>
                <a href="https://test.com">Test Site</a>
            </body>
            </html>
        "#;

        let bookmarks = parse_safari_html(html).unwrap();
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].title, "Example");
        assert_eq!(bookmarks[0].url.as_ref().unwrap(), "https://example.com");
        assert_eq!(bookmarks[1].title, "Test Site");
    }

    #[test]
    fn test_collect_homepages_recursive() {
        let mut bookmarks = vec![
            create_bookmark("1", "Homepage", Some("https://example.com")),
            create_bookmark("2", "Article", Some("https://example.com/article")),
            create_folder(
                "3",
                "Folder",
                vec![
                    create_bookmark("4", "SubHomepage", Some("https://sub.example.com/")),
                    create_bookmark("5", "SubArticle", Some("https://sub.example.com/page")),
                ],
            ),
        ];

        let mut collected = Vec::new();
        SyncEngine::collect_homepages_recursive(&mut bookmarks, &mut collected);

        assert_eq!(collected.len(), 2); // Two homepages
        assert_eq!(SyncEngine::count_all_bookmarks(&bookmarks), 2); // Two non-homepages remain
    }

    #[test]
    fn test_cleanup_stats_default() {
        let stats = CleanupStats::default();
        assert_eq!(stats.duplicates_removed, 0);
        assert_eq!(stats.empty_folders_removed, 0);
    }

    #[test]
    fn test_classification_rule_matches_url_pattern() {
        let rule = ClassificationRule::new(
            "login",
            "登录入口",
            "Login",
            vec!["login", "signin"],
            vec![],
            vec![],
            vec![],
            100,
            "Login pages",
        );

        assert!(rule.matches("https://example.com/login", "Example"));
        assert!(rule.matches("https://signin.example.com", "Example"));
        assert!(!rule.matches("https://example.com/home", "Example"));
    }

    #[test]
    fn test_classification_rule_matches_domain_pattern() {
        let rule = ClassificationRule::new(
            "social",
            "社交媒体",
            "Social",
            vec![],
            vec!["twitter.com", "facebook.com"],
            vec![],
            vec![],
            90,
            "Social media",
        );

        assert!(rule.matches("https://twitter.com/user", "Twitter"));
        assert!(rule.matches("https://facebook.com/page", "Facebook"));
        assert!(!rule.matches("https://example.com", "Example"));
    }

    #[test]
    fn test_classification_rule_matches_path_pattern() {
        let rule = ClassificationRule::new(
            "admin",
            "管理后台",
            "Admin",
            vec![],
            vec![],
            vec!["/admin", "/dashboard"],
            vec![],
            80,
            "Admin pages",
        );

        assert!(rule.matches("https://example.com/admin/users", "Admin Panel"));
        assert!(rule.matches("https://example.com/dashboard", "Dashboard"));
        assert!(!rule.matches("https://example.com/home", "Home"));
    }

    #[test]
    fn test_classification_rule_matches_title_pattern() {
        let rule = ClassificationRule::new(
            "docs",
            "文档参考",
            "Docs",
            vec![],
            vec![],
            vec![],
            vec!["文档", "documentation"],
            70,
            "Documentation",
        );

        assert!(rule.matches("https://example.com", "API 文档"));
        assert!(rule.matches("https://example.com", "Documentation Guide"));
        assert!(!rule.matches("https://example.com", "Home Page"));
    }

    #[test]
    fn test_classification_rule_case_insensitive() {
        let rule = ClassificationRule::new(
            "test",
            "测试",
            "Test",
            vec!["LOGIN"],
            vec!["GITHUB.COM"],
            vec![],
            vec![],
            100,
            "Test",
        );

        assert!(rule.matches("https://example.com/login", "Test"));
        assert!(rule.matches("https://github.com/repo", "Test"));
    }

    #[test]
    fn test_get_builtin_rules() {
        let rules = get_builtin_rules();

        assert!(rules.len() >= 18);

        let login_rule = rules.iter().find(|r| r.name == "login");
        assert!(login_rule.is_some());
        assert_eq!(login_rule.unwrap().folder_name, "登录入口");

        let social_rule = rules.iter().find(|r| r.name == "social");
        assert!(social_rule.is_some());
    }

    #[test]
    fn test_classification_stats_default() {
        let stats = ClassificationStats::default();
        assert_eq!(stats.total_processed, 0);
        assert_eq!(stats.total_classified, 0);
        assert_eq!(stats.unclassified, 0);
        assert!(stats.by_category.is_empty());
    }

    #[test]
    fn test_rule_priority_order() {
        let mut rules = get_builtin_rules();
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Login should have highest priority (100)
        assert_eq!(rules[0].name, "login");
        assert_eq!(rules[0].priority, 100);
    }

    #[test]
    fn test_domain_exact_match_no_substring() {
        // 精确匹配: github.com 不应该匹配 mygithub.com
        assert!(!ClassificationRule::match_domain_exact(
            "mygithub.com",
            "github.com"
        ));
        assert!(!ClassificationRule::match_domain_exact(
            "notgithub.com",
            "github.com"
        ));
        assert!(ClassificationRule::match_domain_exact(
            "github.com",
            "github.com"
        ));
    }

    #[test]
    fn test_domain_wildcard_match() {
        // 通配符匹配: *.github.com
        assert!(ClassificationRule::match_domain_exact(
            "gist.github.com",
            "*.github.com"
        ));
        assert!(ClassificationRule::match_domain_exact(
            "api.github.com",
            "*.github.com"
        ));
        assert!(ClassificationRule::match_domain_exact(
            "github.com",
            "*.github.com"
        ));
        // 不应该匹配不相关的域名
        assert!(!ClassificationRule::match_domain_exact(
            "fakegithub.com",
            "*.github.com"
        ));
    }
}

/// Property-based tests for organize-accuracy-improvement
#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // **Feature: organize-accuracy-improvement, Property 1: 精确域名匹配**
    // **Validates: Requirements 1.1, 1.3**
    // 对于任意域名 D 和模式 P，如果 P 不包含通配符，则 D 必须完全等于 P 才能匹配
    proptest! {
        #[test]
        fn prop_exact_domain_match_no_substring(
            base in "[a-z]{3,8}",
            tld in prop_oneof!["com", "org", "net", "io"],
            prefix in "[a-z]{1,5}",
        ) {
            let domain = format!("{base}.{tld}");
            let prefixed_domain = format!("{prefix}{base}.{tld}");

            // 精确匹配模式不应该匹配带前缀的域名
            prop_assert!(!ClassificationRule::match_domain_exact(&prefixed_domain, &domain));
            // 但应该匹配自身
            prop_assert!(ClassificationRule::match_domain_exact(&domain, &domain));
        }

        #[test]
        fn prop_exact_domain_case_insensitive(
            base in "[a-zA-Z]{3,8}",
            tld in prop_oneof!["com", "org", "net"],
        ) {
            let domain = format!("{base}.{tld}");
            let domain_lower = domain.to_lowercase();

            // 大小写不敏感匹配 (调用者负责转换为小写，这里验证小写匹配)
            prop_assert!(ClassificationRule::match_domain_exact(&domain_lower, &domain_lower));
        }
    }

    // **Feature: organize-accuracy-improvement, Property 2: 通配符域名匹配**
    // **Validates: Requirements 1.2**
    // 对于任意域名 D 和通配符模式 *.suffix，D 匹配当且仅当 D == suffix 或 D 以 .suffix 结尾
    proptest! {
        #[test]
        fn prop_wildcard_domain_match_subdomain(
            subdomain in "[a-z]{2,6}",
            base in "[a-z]{3,8}",
            tld in prop_oneof!["com", "org", "net"],
        ) {
            let suffix = format!("{base}.{tld}");
            let pattern = format!("*.{suffix}");
            let full_domain = format!("{subdomain}.{suffix}");

            // 子域名应该匹配通配符模式
            prop_assert!(ClassificationRule::match_domain_exact(&full_domain, &pattern));
            // 基础域名也应该匹配
            prop_assert!(ClassificationRule::match_domain_exact(&suffix, &pattern));
        }

        #[test]
        fn prop_wildcard_domain_no_match_unrelated(
            unrelated in "[a-z]{3,8}",
            base in "[a-z]{3,8}",
            tld in prop_oneof!["com", "org", "net"],
        ) {
            prop_assume!(unrelated != base);

            let pattern = format!("*.{base}.{tld}");
            let unrelated_domain = format!("{unrelated}.{tld}");

            // 不相关的域名不应该匹配
            prop_assert!(!ClassificationRule::match_domain_exact(&unrelated_domain, &pattern));
        }
    }

    // **Feature: organize-accuracy-improvement, Property 3: 路径前缀匹配**
    // **Validates: Requirements 2.1**
    // 对于任意 URL 路径和前缀列表，如果路径以任一前缀开头，则匹配成功
    proptest! {
        #[test]
        fn prop_path_prefix_match(
            prefix in "/[a-z]{1,5}/",
            suffix in "[a-z]{1,10}",
        ) {
            let full_path = format!("{prefix}{suffix}");
            let url = format!("https://example.com{full_path}");

            let rule = ClassificationRule::new(
                "test", "测试", "Test",
                vec![],
                vec!["example.com"],
                vec![&prefix],
                vec![],
                50,
                "Test rule",
            );

            // 路径以前缀开头应该匹配
            prop_assert!(rule.matches(&url, "Test"));
        }

        #[test]
        fn prop_path_prefix_no_match_middle(
            prefix in "/[a-z]{2,5}/",
            before in "/[a-z]{1,3}",
        ) {
            // 前缀出现在路径中间，不应该匹配
            let path = format!("{before}{prefix}");
            prop_assume!(!path.starts_with(&prefix));
            let url = format!("https://example.com{path}");

            let rule = ClassificationRule::new(
                "test", "测试", "Test",
                vec![],
                vec!["example.com"],
                vec![&prefix],
                vec![],
                50,
                "Test rule",
            );

            let _ = rule.matches(&url, "Test");

            // 前缀不在开头，不应该匹配（除非域名匹配）
            // 注意：由于域名匹配，这里会返回 true，所以我们测试不同的域名
            let url_other = format!("https://other.com{path}");
            let rule_no_domain = ClassificationRule::new(
                "test", "测试", "Test",
                vec![],
                vec![],
                vec![&prefix],
                vec![],
                50,
                "Test rule",
            );

            prop_assert!(!rule_no_domain.matches(&url_other, "Test"));
        }
    }

    // **Feature: organize-accuracy-improvement, Property 7: 规则排序正确性**
    // **Validates: Requirements 6.1, 6.2**
    // 对于任意规则列表，排序后应按 priority 降序，相同 priority 按 specificity 降序
    proptest! {
        #[test]
        fn prop_rule_sorting_by_priority_then_specificity(
            p1 in 1..100i32,
            p2 in 1..100i32,
            s1 in 0..200i32,
            s2 in 0..200i32,
        ) {
            let mut rules = [ClassificationRule {
                    name: "rule1".to_string(),
                    folder_name: "文件夹1".to_string(),
                    folder_name_en: "Folder1".to_string(),
                    url_patterns: vec![],
                    domain_patterns: vec![],
                    path_patterns: vec![],
                    title_patterns: vec![],
                    priority: p1,
                    description: "Rule 1".to_string(),
                    specificity: s1,
                },
                ClassificationRule {
                    name: "rule2".to_string(),
                    folder_name: "文件夹2".to_string(),
                    folder_name_en: "Folder2".to_string(),
                    url_patterns: vec![],
                    domain_patterns: vec![],
                    path_patterns: vec![],
                    title_patterns: vec![],
                    priority: p2,
                    description: "Rule 2".to_string(),
                    specificity: s2,
                }];

            // 排序
            rules.sort_by(|a, b| {
                match b.priority.cmp(&a.priority) {
                    std::cmp::Ordering::Equal => b.specificity.cmp(&a.specificity),
                    other => other,
                }
            });

            // 验证排序正确性
            if rules[0].priority == rules[1].priority {
                // 相同 priority，按 specificity 降序
                prop_assert!(rules[0].specificity >= rules[1].specificity);
            } else {
                // 不同 priority，按 priority 降序
                prop_assert!(rules[0].priority > rules[1].priority);
            }
        }
    }

    // **Feature: organize-accuracy-improvement, Property 8: Specificity 计算正确性**
    // **Validates: Requirements 6.3**
    // 路径匹配的 specificity > 域名匹配的 specificity > 标题匹配的 specificity
    proptest! {
        #[test]
        fn prop_specificity_calculation_order(
            _dummy in 0..10i32,  // 只是为了让 proptest 运行多次
        ) {
            // 只有路径
            let path_only = ClassificationRule::new(
                "path", "路径", "Path",
                vec![],
                vec![],
                vec!["/test/"],
                vec![],
                50,
                "Path only",
            );

            // 只有域名
            let domain_only = ClassificationRule::new(
                "domain", "域名", "Domain",
                vec![],
                vec!["example.com"],
                vec![],
                vec![],
                50,
                "Domain only",
            );

            // 只有标题
            let title_only = ClassificationRule::new(
                "title", "标题", "Title",
                vec![],
                vec![],
                vec![],
                vec!["keyword"],
                50,
                "Title only",
            );

            // 验证: 路径 > 域名 > 标题
            prop_assert!(path_only.specificity > domain_only.specificity);
            prop_assert!(domain_only.specificity > title_only.specificity);

            // 验证具体值
            prop_assert_eq!(path_only.specificity, 100);
            prop_assert_eq!(domain_only.specificity, 50);
            prop_assert_eq!(title_only.specificity, 10);
        }
    }

    // **Feature: organize-accuracy-improvement, Property 9: 更具体规则优先**
    // **Validates: Requirements 1.4, 2.3**
    // 对于任意书签和多个匹配规则，应选择 specificity 最高的规则
    proptest! {
        #[test]
        fn prop_more_specific_rule_wins(
            priority in 50..100i32,
        ) {
            // 创建两个规则：一个只有域名，一个有域名+路径
            let domain_rule = ClassificationRule::new(
                "domain_only", "域名规则", "Domain Rule",
                vec![],
                vec!["example.com"],
                vec![],
                vec![],
                priority,
                "Domain only rule",
            );

            let path_rule = ClassificationRule::new(
                "domain_path", "路径规则", "Path Rule",
                vec![],
                vec!["example.com"],
                vec!["/docs/"],
                vec![],
                priority,  // 相同优先级
                "Domain + path rule",
            );

            // 路径规则应该有更高的 specificity
            prop_assert!(path_rule.specificity > domain_rule.specificity);

            // 排序后，路径规则应该排在前面（相同 priority 时按 specificity 降序）
            let mut rules = [domain_rule, path_rule];
            rules.sort_by(|a, b| {
                match b.priority.cmp(&a.priority) {
                    std::cmp::Ordering::Equal => b.specificity.cmp(&a.specificity),
                    other => other,
                }
            });

            prop_assert_eq!(&rules[0].name, "domain_path");
            prop_assert_eq!(&rules[1].name, "domain_only");
        }
    }
}

impl SyncEngine {
    /// Set hub browsers with Firefox Sync integration
    pub async fn set_hub_browsers_with_firefox_sync(
        &mut self,
        config: &HubConfig,
        firefox_sync_strategy: crate::firefox_sync::SyncStrategy,
    ) -> Result<()> {
        let hub_names = &config.hub_names;
        let sync_history = config.sync_history;
        let sync_reading_list = config.sync_reading_list;
        let sync_cookies = config.sync_cookies;
        let clear_others = config.clear_others;
        let dry_run = config.dry_run;
        let verbose = config.verbose;
        use crate::firefox_sync::{FirefoxSyncHandler, SyncStrategy};
        use crate::firefox_sync_api::FirefoxSyncAPIClient;

        // 检测Waterfox profile
        let waterfox_profile = PathBuf::from(std::env::var("HOME")?)
            .join("Library/Application Support/Waterfox/Profiles/ll4fbmm0.default-release");

        // 如果使用API策略，先检查是否能加载API客户端
        let api_client = if matches!(firefox_sync_strategy, SyncStrategy::UseAPI) {
            if waterfox_profile.exists() {
                match FirefoxSyncAPIClient::from_profile(&waterfox_profile) {
                    Ok(client) => {
                        info!("✅ Firefox Sync API client loaded");
                        Some(client)
                    }
                    Err(e) => {
                        warn!("⚠️  Failed to load Firefox Sync API client: {}", e);
                        warn!("   Falling back to local sync only");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let firefox_sync_handler = if waterfox_profile.exists() && api_client.is_none() {
            Some(FirefoxSyncHandler::new(
                &waterfox_profile,
                firefox_sync_strategy,
            )?)
        } else {
            None
        };

        // 在写入前执行Firefox Sync处理
        if let Some(ref handler) = firefox_sync_handler {
            handler.before_write()?;
        }

        // 执行正常的同步流程
        let hub_config = HubConfig {
            hub_names: hub_names.clone(),
            sync_history,
            sync_reading_list,
            sync_cookies,
            clear_others,
            dry_run,
            verbose,
        };
        self.set_hub_browsers(&hub_config).await?;

        // 如果使用API策略，上传到云端
        if let Some(ref client) = api_client {
            if !dry_run {
                info!("");
                info!("📤 Uploading bookmarks to Firefox Sync cloud via API...");

                // 读取刚写入的书签
                for adapter in &self.adapters {
                    if adapter
                        .browser_type()
                        .name()
                        .to_lowercase()
                        .contains("waterfox")
                    {
                        if let Ok(bookmarks) = adapter.read_bookmarks() {
                            match client.upload_bookmarks(&bookmarks).await {
                                Ok(()) => {
                                    info!("✅ Bookmarks uploaded to cloud successfully!");
                                    info!("   Your changes are now synced across all devices");
                                }
                                Err(e) => {
                                    warn!("⚠️  Failed to upload to cloud: {}", e);
                                    warn!("   Local changes are saved, but not synced to cloud");
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // 在写入后执行Firefox Sync处理（非API策略）
        if let Some(ref handler) = firefox_sync_handler {
            if !dry_run {
                handler.after_write()?;
            }
        }

        Ok(())
    }
}

impl SyncEngine {
    /// Migrate all data to Safari and optionally clear other browsers
    pub async fn migrate_to_safari(
        &mut self,
        dry_run: bool,
        keep_source: bool,
        verbose: bool,
    ) -> Result<()> {
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("📖 Phase 1: Reading all browser data");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Collect all bookmarks
        let mut all_bookmarks: HashMap<BrowserType, Vec<Bookmark>> = HashMap::new();
        let mut all_history: HashMap<BrowserType, Vec<HistoryItem>> = HashMap::new();
        let mut all_reading_lists: HashMap<BrowserType, Vec<ReadingListItem>> = HashMap::new();

        for adapter in &self.adapters {
            let browser_name = adapter.browser_type().name();

            // Read bookmarks
            if let Ok(bookmarks) = adapter.read_bookmarks() {
                let count = Self::count_all_bookmarks(&bookmarks);
                if count > 0 {
                    info!("  {} : {} bookmarks", browser_name, count);
                    all_bookmarks.insert(adapter.browser_type(), bookmarks);
                }
            }

            // Read history
            if adapter.supports_history() {
                if let Ok(history) = adapter.read_history(None) {
                    if !history.is_empty() {
                        info!("  {} : {} history items", browser_name, history.len());
                        all_history.insert(adapter.browser_type(), history);
                    }
                }
            }

            // Read reading list
            if adapter.supports_reading_list() {
                if let Ok(reading_list) = adapter.read_reading_list() {
                    if !reading_list.is_empty() {
                        info!(
                            "  {} : {} reading list items",
                            browser_name,
                            reading_list.len()
                        );
                        all_reading_lists.insert(adapter.browser_type(), reading_list);
                    }
                }
            }
        }

        // Merge data
        info!("");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("🔄 Phase 2: Merging and deduplicating");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let mut merged_bookmarks = self.merge_bookmarks(&all_bookmarks, verbose)?;
        let merged_history = self.merge_history(&all_history, verbose)?;
        let merged_reading_list = self.merge_reading_lists(&all_reading_lists, verbose)?;

        // Thoroughly clean empty folders (iterate until none remain)
        info!("🧹 Phase 2.5: Cleaning empty folders");
        let mut total_empty_removed = 0;
        loop {
            let removed = Self::cleanup_empty_folders(&mut merged_bookmarks);
            if removed == 0 {
                break;
            }
            total_empty_removed += removed;
        }
        if total_empty_removed > 0 {
            info!("   Removed {} empty folders", total_empty_removed);
        }

        let bookmark_count = Self::count_all_bookmarks(&merged_bookmarks);
        let folder_count = Self::count_all_folders(&merged_bookmarks);

        info!(
            "  📚 Merged bookmarks: {} URLs, {} folders",
            bookmark_count, folder_count
        );
        info!("  📜 Merged history: {} items", merged_history.len());
        info!(
            "  📖 Merged reading list: {} items",
            merged_reading_list.len()
        );

        if dry_run {
            info!("");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("🏃 Dry Run Mode - Actions that would be performed:");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("  ✅ Write {} bookmarks to Safari", bookmark_count);
            info!(
                "  ✅ Write {} history items to Safari",
                merged_history.len()
            );
            info!(
                "  ✅ Write {} reading list items to Safari",
                merged_reading_list.len()
            );
            if !keep_source {
                info!("  🗑️  Clear bookmarks, history, reading list from other browsers");
            }
            return Ok(());
        }

        // Write to Safari
        info!("");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("💾 Phase 3: Writing to Safari");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        for adapter in &self.adapters {
            if adapter.browser_type() == BrowserType::Safari {
                let backup_path = adapter
                    .backup_bookmarks()
                    .context("Refusing to write Safari bookmarks: backup failed")?;
                info!("  💾 Safari backup: {:?}", backup_path);

                // Write bookmarks
                if let Err(e) = adapter.write_bookmarks(&merged_bookmarks) {
                    warn!("  ⚠️  Failed to write Safari bookmarks: {}", e);
                } else {
                    info!("  ✅ Wrote {} bookmarks to Safari", bookmark_count);
                }

                // Write reading list
                if adapter.supports_reading_list() {
                    if let Err(e) = adapter.write_reading_list(&merged_reading_list) {
                        warn!("  ⚠️  Failed to write Safari reading list: {}", e);
                    } else {
                        info!(
                            "  ✅ Wrote {} reading list items to Safari",
                            merged_reading_list.len()
                        );
                    }
                }

                // Safari history is managed by the system
                info!("  ℹ️  Safari history is managed by the system, cannot write directly");

                break;
            }
        }

        // Clear other browsers
        if !keep_source {
            info!("");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("🗑️  Phase 4: Clearing other browsers");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            for adapter in &self.adapters {
                if adapter.browser_type() == BrowserType::Safari {
                    continue; // Skip Safari
                }

                let browser_name = adapter.browser_type().name();

                let (_, version) =
                    Self::read_single_profile_for_write(adapter.as_ref(), "clear bookmarks from")?;

                let backup_path = adapter
                    .backup_bookmarks()
                    .with_context(|| format!("Refusing to clear {browser_name}: backup failed"))?;
                info!("  💾 {} backup: {:?}", browser_name, backup_path);

                // Clear bookmarks (write empty list)
                let empty_bookmarks: Vec<Bookmark> = vec![];
                let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
                if let Err(e) = adapter.write_bookmarks(&empty_bookmarks) {
                    warn!("  ⚠️  Failed to clear {} bookmarks: {}", browser_name, e);
                } else {
                    info!("  ✅ Cleared {} bookmarks", browser_name);
                }

                // Clear history
                if adapter.supports_history() {
                    let empty_history: Vec<HistoryItem> = vec![];
                    if let Err(e) = adapter.write_history(&empty_history) {
                        warn!("  ⚠️  Failed to clear {} history: {}", browser_name, e);
                    } else {
                        info!("  ✅ Cleared {} history", browser_name);
                    }
                }

                // Clear reading list
                if adapter.supports_reading_list() {
                    let empty_reading_list: Vec<ReadingListItem> = vec![];
                    if let Err(e) = adapter.write_reading_list(&empty_reading_list) {
                        warn!("  ⚠️  Failed to clear {} reading list: {}", browser_name, e);
                    } else {
                        info!("  ✅ Cleared {} reading list", browser_name);
                    }
                }
            }
        }

        // Verify
        info!("");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("🔍 Phase 5: Verification");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        for adapter in &self.adapters {
            if adapter.browser_type() == BrowserType::Safari {
                if let Ok(bookmarks) = adapter.read_bookmarks() {
                    let count = Self::count_all_bookmarks(&bookmarks);
                    info!("  Safari: {} bookmarks", count);
                }
            }
        }

        info!("");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("📊 Migration complete!");
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("  Safari now contains all browser data");
        if !keep_source {
            info!("  Other browser data has been cleared (backups saved)");
        }

        Ok(())
    }
}

impl SyncEngine {
    /// Analyze bookmarks for anomalies
    pub async fn analyze_bookmarks(&self, browser_names: Option<&str>) -> Result<()> {
        use crate::cleanup::detect_anomalies;

        info!("🔍 Analyzing bookmark anomalies...");

        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> =
                names.split(',').map(|s| s.trim().to_lowercase()).collect();

            self.adapters
                .iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    browser_list.iter().any(|b| name.contains(b))
                })
                .collect()
        } else {
            self.adapters.iter().collect()
        };

        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();

            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    let total = Self::count_all_bookmarks(&bookmarks);
                    let folders = Self::count_all_folders(&bookmarks);

                    println!("\n📊 {browser_name} Bookmark Analysis");
                    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                    println!("  Total bookmarks: {total}");
                    println!("  Total folders: {folders}");

                    let report = detect_anomalies(&bookmarks);
                    report.print_summary();
                }
                Err(e) => {
                    warn!("⚠️  Cannot read {} bookmarks: {}", browser_name, e);
                }
            }
        }

        Ok(())
    }

    // deep_clean_bookmarks 已移除 - 自动删除功能误删风险太高

    /// Export all bookmarks from specified browsers to HTML file
    /// This is the RECOMMENDED way to export bookmarks - let users import manually
    pub async fn export_to_html(
        &self,
        browser_names: Option<&str>,
        output_path: &str,
        config: &ExportConfig,
    ) -> Result<usize> {
        self.export_to_html_with_extra(browser_names, output_path, config, Vec::new())
            .await
    }

    fn requests_all_browsers(browser_names: Option<&str>) -> bool {
        browser_names.is_none_or(|names| {
            names
                .split(',')
                .any(|name| name.trim().eq_ignore_ascii_case("all"))
        })
    }

    fn legacy_bookmark_from_node(node: BookmarkNode) -> Option<Bookmark> {
        match node {
            BookmarkNode::Folder(folder) => Some(Bookmark {
                id: folder.id,
                title: folder.title,
                url: None,
                folder: true,
                children: folder
                    .children
                    .into_iter()
                    .filter_map(Self::legacy_bookmark_from_node)
                    .collect(),
                date_added: folder.date_added.map(|date| date.timestamp_millis()),
                date_modified: folder.date_modified.map(|date| date.timestamp_millis()),
            }),
            BookmarkNode::Bookmark(bookmark) => Some(Bookmark {
                id: bookmark.id,
                title: bookmark.title,
                url: Some(bookmark.raw_url),
                folder: false,
                children: Vec::new(),
                date_added: bookmark.date_added.map(|date| date.timestamp_millis()),
                date_modified: bookmark.date_modified.map(|date| date.timestamp_millis()),
            }),
            BookmarkNode::Separator(_) => None,
        }
    }

    fn legacy_bookmarks_from_tree(tree: BookmarkTree, merge: bool) -> Vec<Bookmark> {
        let roots: Vec<Bookmark> = tree
            .roots
            .into_iter()
            .filter_map(Self::legacy_bookmark_from_node)
            .collect();
        if merge {
            roots
                .into_iter()
                .flat_map(|root| {
                    if root.folder {
                        root.children
                    } else {
                        vec![root]
                    }
                })
                .collect()
        } else {
            roots
        }
    }

    fn collect_comprehensive_profiles(
        merge: bool,
        exclude_onion: bool,
    ) -> Result<(Vec<Bookmark>, InventoryReport)> {
        let snapshots = tempfile::tempdir().context("create isolated profile snapshots")?;
        let (tree, inventory, _) = collector::BookmarkCollector::collect_all_with_options(
            snapshots.path(),
            exclude_onion,
        )?;
        Ok((Self::legacy_bookmarks_from_tree(tree, merge), inventory))
    }

    /// Collect bookmarks from browsers for export (without writing to file)
    pub async fn collect_bookmarks_for_export(
        &self,
        browser_names: Option<&str>,
        config: &ExportConfig,
        mut extra_bookmarks: Vec<Bookmark>,
    ) -> Result<Vec<Bookmark>> {
        let merge = config.merge;

        if config.exclude_onion {
            remove_onion_bookmarks(&mut extra_bookmarks);
        }
        let mut all_bookmarks = extra_bookmarks;

        if config.all_profiles && Self::requests_all_browsers(browser_names) {
            let (bookmarks, _) = Self::collect_comprehensive_profiles(merge, config.exclude_onion)?;
            all_bookmarks.extend(bookmarks);
            return Ok(all_bookmarks);
        }

        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> =
                names.split(',').map(|s| s.trim().to_lowercase()).collect();
            self.adapters
                .iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    let name_normalized = name.replace([' ', '_'], "-");
                    browser_list.iter().any(|b| {
                        let b_normalized = b.replace([' ', '_'], "-");
                        name_normalized == b_normalized
                            || name.contains(b)
                            || b.contains(&name)
                            || (b.contains("nightly") && name.contains("nightly"))
                            || (b == "all")
                    })
                })
                .collect()
        } else {
            self.adapters.iter().collect()
        };
        if target_adapters.is_empty() {
            anyhow::bail!("No matching browsers found");
        }

        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();
            let res = if config.all_profiles {
                adapter.read_all_bookmarks()
            } else {
                adapter.read_bookmarks()
            };
            match res {
                Ok(mut bookmarks) => {
                    if config.exclude_onion {
                        remove_onion_bookmarks(&mut bookmarks);
                    }
                    if merge {
                        all_bookmarks.extend(bookmarks);
                    } else {
                        let browser_folder = Bookmark {
                            id: format!(
                                "browser-{}",
                                browser_name.to_lowercase().replace(' ', "-")
                            ),
                            title: browser_name.to_string(),
                            url: None,
                            folder: true,
                            children: bookmarks,
                            date_added: Some(chrono::Utc::now().timestamp_millis()),
                            date_modified: None,
                        };
                        all_bookmarks.push(browser_folder);
                    }
                }
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("Cannot collect every selected profile from {browser_name}")
                    });
                }
            }
        }

        Ok(all_bookmarks)
    }

    /// Export all bookmarks with additional bookmarks from external sources and generate a detailed JSON report
    pub async fn export_to_html_with_report(
        &self,
        browser_names: Option<&str>,
        output_path: &str,
        report_json_path: Option<&str>,
        config: &ExportConfig,
        mut extra_bookmarks: Vec<Bookmark>,
        external_source_reports: Vec<serde_json::Value>,
    ) -> Result<usize> {
        let merge = config.merge;
        let deduplicate = config.deduplicate;
        let clean_empty = config.clean_empty;
        let verbose = config.verbose;
        info!("📤 Exporting bookmarks to HTML...");

        let comprehensive = config.all_profiles && Self::requests_all_browsers(browser_names);
        // Determine target browsers
        let target_adapters: Vec<_> = if comprehensive {
            Vec::new()
        } else if let Some(names) = browser_names {
            let browser_list: Vec<String> =
                names.split(',').map(|s| s.trim().to_lowercase()).collect();

            self.adapters
                .iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    let name_normalized = name.replace([' ', '_'], "-");
                    browser_list.iter().any(|b| {
                        let b_normalized = b.replace([' ', '_'], "-");
                        // Exact match first
                        name_normalized == b_normalized ||
                        // Then partial match
                        name.contains(b) || b.contains(&name) ||
                        // Special case for "nightly" variants
                        (b.contains("nightly") && name.contains("nightly")) ||
                        // All browsers
                        (b == "all")
                    })
                })
                .collect()
        } else {
            self.adapters.iter().collect()
        };

        if !comprehensive && target_adapters.is_empty() {
            anyhow::bail!("No matching browsers found");
        }

        info!("🎯 Target browsers:");
        if comprehensive {
            info!("  - all discovered browser installations and profiles");
        } else {
            for adapter in &target_adapters {
                info!("  - {}", adapter.browser_type().name());
            }
        }

        let mut all_bookmarks: Vec<Bookmark> = Vec::new();
        let mut browser_stats: Vec<(String, usize)> = Vec::new();
        let mut browser_source_reports: Vec<serde_json::Value> = Vec::new();
        let mut inventory_warnings: Vec<String> = Vec::new();

        if config.exclude_onion {
            let removed = remove_onion_bookmarks(&mut extra_bookmarks);
            if removed > 0 {
                info!("  🧅 Excluded {removed} onion bookmark(s) from external sources");
            }
        }
        if !extra_bookmarks.is_empty() {
            let extra_count = Self::count_all_bookmarks(&extra_bookmarks);
            info!("  📥 HTML / External import: {} bookmarks", extra_count);
            browser_stats.push(("External Import".to_string(), extra_count));
            all_bookmarks.extend(extra_bookmarks);
        }

        if comprehensive {
            let (bookmarks, inventory) =
                Self::collect_comprehensive_profiles(merge, config.exclude_onion)?;
            inventory_warnings = inventory.warnings.clone();
            for warning in &inventory.warnings {
                warn!("  ⚠️  {warning}");
            }
            for installation in inventory.installations {
                let browser_name = installation.installation.name;
                let total_browser_bookmarks: usize = installation
                    .profiles
                    .iter()
                    .map(|profile| profile.bookmark_count)
                    .sum();
                let total_browser_folders: usize = installation
                    .profiles
                    .iter()
                    .map(|profile| profile.folder_count)
                    .sum();
                let profile_reports: Vec<_> = installation
                    .profiles
                    .into_iter()
                    .map(|profile| {
                        serde_json::json!({
                            "profile_id": profile.profile.id,
                            "profile_name": profile.profile.name,
                            "bookmark_count": profile.bookmark_count,
                            "folder_count": profile.folder_count,
                            "read_success": profile.read_success,
                            "error": profile.error,
                            "source_path": profile.profile.path,
                            "snapshot_retained": false,
                        })
                    })
                    .collect();
                browser_stats.push((browser_name.clone(), total_browser_bookmarks));
                browser_source_reports.push(serde_json::json!({
                    "browser_name": browser_name,
                    "discovery_error": installation.discovery_error,
                    "profiles": profile_reports,
                    "total_bookmarks_collected": total_browser_bookmarks,
                    "total_folders_collected": total_browser_folders
                }));
            }
            all_bookmarks.extend(bookmarks);
        } else {
            for adapter in &target_adapters {
                let browser_name = adapter.browser_type().name();
                let profiles_res = if config.all_profiles {
                    adapter.read_all_bookmarks_with_profiles()
                } else {
                    adapter
                        .read_bookmarks()
                        .map(|b| vec![("Default".to_string(), b)])
                };
                match profiles_res {
                    Ok(profiles) => {
                        let mut profile_reports = Vec::new();
                        let mut total_browser_bookmarks = 0;
                        let mut total_browser_folders = 0;
                        let mut browser_bookmarks = Vec::new();
                        for (p_name, mut p_bookmarks) in profiles {
                            if config.exclude_onion {
                                remove_onion_bookmarks(&mut p_bookmarks);
                            }
                            let b_count = Self::count_all_bookmarks(&p_bookmarks);
                            let f_count = Self::count_all_folders(&p_bookmarks);
                            total_browser_bookmarks += b_count;
                            total_browser_folders += f_count;
                            let top_folders: Vec<String> =
                                p_bookmarks.iter().map(|b| b.title.clone()).collect();
                            info!("  ✅ {} ({}) : {} bookmarks", browser_name, p_name, b_count);
                            profile_reports.push(serde_json::json!({
                                "profile_name": p_name,
                                "bookmark_count": b_count,
                                "folder_count": f_count,
                                "top_level_folders": top_folders,
                                "folder_breakdown": Self::get_folder_breakdown(&p_bookmarks)
                            }));
                            browser_bookmarks.extend(p_bookmarks);
                        }
                        browser_stats.push((browser_name.to_string(), total_browser_bookmarks));
                        browser_source_reports.push(serde_json::json!({
                            "browser_name": browser_name,
                            "profiles": profile_reports,
                            "total_bookmarks_collected": total_browser_bookmarks,
                            "total_folders_collected": total_browser_folders
                        }));

                        if merge {
                            all_bookmarks.extend(browser_bookmarks);
                        } else {
                            let browser_folder = Bookmark {
                                id: format!(
                                    "browser-{}",
                                    browser_name.to_lowercase().replace(' ', "-")
                                ),
                                title: browser_name.to_string(),
                                url: None,
                                folder: true,
                                children: browser_bookmarks,
                                date_added: Some(chrono::Utc::now().timestamp_millis()),
                                date_modified: None,
                            };
                            all_bookmarks.push(browser_folder);
                        }
                    }
                    Err(e) => {
                        return Err(e).with_context(|| {
                            format!("Cannot collect every selected profile from {browser_name}")
                        });
                    }
                }
            }
        }

        // Apply folder filter if specified
        if let Some(ref folder_name) = config.folder_filter {
            info!("📁 Filtering by folder: \"{}\"", folder_name);
            let filtered = Self::extract_bookmarks_from_folder(&all_bookmarks, folder_name);
            let filtered_count = Self::count_all_bookmarks(&filtered);
            info!(
                "  ✅ Found {} bookmarks in folders matching \"{}\"",
                filtered_count, folder_name
            );
            all_bookmarks = filtered;
        }

        let before_dedup = Self::count_all_bookmarks(&all_bookmarks);
        let before_dedup_folders = Self::count_all_folders(&all_bookmarks);
        info!("\n📊 Collection complete: {} bookmarks", before_dedup);

        let mut removed_dups = 0;
        // Deduplicate if requested
        if deduplicate {
            info!("🧹 Deduplicating...");
            Self::deduplicate_bookmarks_global(&mut all_bookmarks);
            let after_dedup = Self::count_all_bookmarks(&all_bookmarks);
            removed_dups = before_dedup.saturating_sub(after_dedup);
            if removed_dups > 0 {
                info!("  ✅ Removed {} duplicate bookmarks", removed_dups);
            }
        }

        let mut total_empty_removed = 0;
        // Clean empty folders if requested
        if clean_empty {
            info!("🧹 Cleaning empty folders...");
            loop {
                let removed = Self::cleanup_empty_folders(&mut all_bookmarks);
                if removed == 0 {
                    break;
                }
                total_empty_removed += removed;
            }
            if total_empty_removed > 0 {
                info!("  ✅ Removed {} empty folders", total_empty_removed);
            }
        }

        // Flatten export: remove browser root folders
        if config.flat {
            info!("📦 Flattening: removing browser root folders...");
            let flat_config = FlatExportConfig {
                flatten_root: true,
                root_folders_to_remove: None,
            };
            let (flattened, stats) = flatten_bookmarks(&all_bookmarks, &flat_config);
            all_bookmarks = flattened;
            if stats.root_folders_removed > 0 {
                info!(
                    "  ✅ Removed {} browser root folders, promoted {} items",
                    stats.root_folders_removed, stats.bookmarks_promoted
                );
            }
        }

        // Wrap all bookmarks in a single root folder (default behavior)
        // Use wrap_folder name or default "📁镜像文件夹"
        let wrap_name = config
            .wrap_folder
            .clone()
            .unwrap_or_else(|| "📁镜像文件夹".to_string());

        if config.no_wrap {
            // No wrap - just ensure protected folders exist
            ensure_protected_folders(&mut all_bookmarks);
        } else {
            info!(
                "📦 Wrapping all bookmarks in root folder: \"{}\"",
                wrap_name
            );

            // Extract and MERGE all protected folders (👀临时) into one
            let mut merged_temp_children: Vec<Bookmark> = Vec::new();
            all_bookmarks.retain(|b| {
                if b.folder
                    && PROTECTED_FOLDERS
                        .iter()
                        .any(|p| b.title.to_lowercase().contains(&p.to_lowercase()))
                {
                    // Merge children into single temp folder
                    merged_temp_children.extend(b.children.clone());
                    false // Remove from all_bookmarks
                } else {
                    true // Keep in all_bookmarks
                }
            });

            let wrapped = Bookmark {
                id: format!("wrap-{}", wrap_name.to_lowercase().replace(' ', "-")),
                title: wrap_name,
                url: None,
                folder: true,
                children: all_bookmarks,
                date_added: Some(chrono::Utc::now().timestamp_millis()),
                date_modified: None,
            };

            // Create single merged temp folder at top level
            let temp_folder = Bookmark {
                id: "temp-folder".to_string(),
                title: "👀临时".to_string(),
                url: None,
                folder: true,
                children: merged_temp_children,
                date_added: Some(chrono::Utc::now().timestamp_millis()),
                date_modified: None,
            };

            // Top level: wrap folder + single merged temp folder
            all_bookmarks = vec![wrapped, temp_folder];
            info!("  ✅ Merged all temp folders into single 👀临时");
        }

        let final_count = Self::count_all_bookmarks(&all_bookmarks);
        let final_folders = Self::count_all_folders(&all_bookmarks);

        // Export to HTML
        let output = if output_path.starts_with("~/") {
            let home = std::env::var("HOME")
                .map_err(|e| anyhow::anyhow!("获取 HOME 环境变量失败: {e}"))?;
            output_path.replacen('~', &home, 1)
        } else {
            output_path.to_string()
        };

        export_bookmarks_to_html(&all_bookmarks, &output)?;

        let report_path_str = if let Some(p) = report_json_path {
            if p.starts_with("~/") {
                let home = std::env::var("HOME")
                    .map_err(|e| anyhow::anyhow!("获取 HOME 环境变量失败: {e}"))?;
                p.replacen('~', &home, 1)
            } else {
                p.to_string()
            }
        } else {
            let p = Path::new(&output);
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
            let dir = p.parent().unwrap_or(Path::new("."));
            dir.join(format!("{stem}_report.json"))
                .to_string_lossy()
                .to_string()
        };
        let external_source_failed = external_source_reports.iter().any(|report| {
            matches!(
                report.get("status").and_then(serde_json::Value::as_str),
                Some("failed" | "unsupported" | "unverified" | "partial")
            )
        });

        let report = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "report_version": "1.0.0",
            "export_configuration": {
                "target_browsers": browser_names.unwrap_or("all"),
                "all_profiles": config.all_profiles,
                "deduplicate_mode": if config.deduplicate { "exact" } else { "none" },
                "merge_flat_structure": config.merge,
                "clean_empty_folders": config.clean_empty,
                "flatten_browser_roots": config.flat,
                "exclude_onion": config.exclude_onion,
                "wrap_root_folder": config.wrap_folder.as_deref().unwrap_or("📁镜像文件夹")
            },
            "sources": {
                "installed_browsers": browser_source_reports,
                "external_files_and_directories": external_source_reports,
                "collection_status": if inventory_warnings.is_empty() && !external_source_failed { "complete" } else { "partial" },
                "inventory_warnings": inventory_warnings,
            },
            "deduplication_and_cleanup_statistics": {
                "total_raw_bookmarks_collected": before_dedup,
                "total_raw_folders_collected": before_dedup_folders,
                "exact_duplicate_bookmarks_removed": removed_dups,
                "empty_folders_cleaned": total_empty_removed,
                "unique_bookmarks_remaining": before_dedup.saturating_sub(removed_dups),
                "final_exported_bookmark_nodes": final_count,
                "final_exported_folders": final_folders
            },
            "validity_check_conclusions": {
                "validation_performed": false,
                "note": "未执行 --validate-copy (若启用将在此记录检测结论)"
            },
            "output_summary": {
                "html_file_path": output,
                "json_report_path": report_path_str,
                "total_unique_bookmarks": final_count,
                "total_unique_folders": final_folders,
                "status": "Success"
            }
        });

        let json_content = serde_json::to_string_pretty(&report)?;
        crate::browser_utils::atomic_write_file(
            Path::new(&report_path_str),
            json_content.as_bytes(),
        )
        .with_context(|| format!("Failed to write JSON report to {report_path_str}"))?;
        info!(
            "📋 Detailed Export JSON Report saved to: {}",
            report_path_str
        );

        info!("\n✅ Export complete!");
        info!("   📄 File: {}", output);
        info!("   📊 Bookmarks: {}", final_count);

        if verbose {
            info!("\n📊 Source statistics:");
            for (browser, count) in &browser_stats {
                info!("   {} : {}", browser, count);
            }
        }

        Ok(final_count)
    }

    /// Export all bookmarks with additional bookmarks from external sources
    pub async fn export_to_html_with_extra(
        &self,
        browser_names: Option<&str>,
        output_path: &str,
        config: &ExportConfig,
        extra_bookmarks: Vec<Bookmark>,
    ) -> Result<usize> {
        self.export_to_html_with_report(
            browser_names,
            output_path,
            None,
            config,
            extra_bookmarks,
            Vec::new(),
        )
        .await
    }

    /// Clear bookmarks from specified browsers (use with caution!)
    pub async fn clear_bookmarks(&mut self, browser_names: &str, dry_run: bool) -> Result<()> {
        info!("🗑️  Clearing browser bookmarks...");

        let browser_list: Vec<String> = browser_names
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .collect();

        let target_adapters: Vec<_> = self
            .adapters
            .iter()
            .filter(|a| {
                let name = a.browser_type().name().to_lowercase();
                browser_list.iter().any(|b| name.contains(b) || b == "all")
            })
            .collect();

        if target_adapters.is_empty() {
            anyhow::bail!("No matching browsers found");
        }

        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();

            if dry_run {
                info!("  🏃 {} : will be cleared (dry-run)", browser_name);
                continue;
            }

            let (_, version) =
                Self::read_single_profile_for_write(adapter.as_ref(), "clear bookmarks from")?;

            // Backup first
            match adapter.backup_bookmarks() {
                Ok(backup_path) => {
                    info!("  💾 {} : backup created {:?}", browser_name, backup_path);
                }
                Err(e) => {
                    return Err(
                        e.context(format!("Refusing to clear {browser_name}: backup failed"))
                    );
                }
            }

            // Write empty bookmarks
            let _write_guard = Self::guard_bookmark_write(version.as_ref())?;
            match adapter.write_bookmarks(&[]) {
                Ok(()) => {
                    info!("  ✅ {} : cleared", browser_name);
                }
                Err(e) => {
                    return Err(e.context(format!("Failed to clear {browser_name} bookmarks")));
                }
            }
        }

        Ok(())
    }

    /// Find the newest backup belonging to exactly this source file.
    fn find_latest_bookmark_backup(source: &Path) -> Result<PathBuf> {
        let parent = source
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid bookmark path: {}", source.display()))?;
        let source_name = source
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid bookmark path: {}", source.display()))?
            .to_string_lossy()
            .into_owned();

        let mut candidates = Vec::new();
        for extension in [
            "sqlite.backup",
            "json.backup",
            "plist.backup",
            "sqlite.cloud_reset_backup",
        ] {
            let candidate = source.with_extension(extension);
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }

        let uuid_prefix = format!("{source_name}.backup-");
        for entry in std::fs::read_dir(parent)? {
            let path = entry?.path();
            if path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&uuid_prefix))
            {
                candidates.push(path);
            }
        }

        candidates.sort_by(|left, right| {
            let left_modified = std::fs::metadata(left)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let right_modified = std::fs::metadata(right)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            left_modified
                .cmp(&right_modified)
                .then_with(|| left.cmp(right))
        });

        candidates
            .pop()
            .ok_or_else(|| anyhow::anyhow!("No backup found for {}", source.display()))
    }

    /// Restore bookmarks from backup
    pub async fn restore_backup(
        &mut self,
        browser_name: &str,
        backup_file: Option<&str>,
    ) -> Result<()> {
        info!("🔄 Restoring bookmark backup...");

        let browser_lower = browser_name.to_lowercase();

        // Find the adapter
        let adapter = self
            .adapters
            .iter()
            .find(|a| {
                a.browser_type()
                    .name()
                    .to_lowercase()
                    .contains(&browser_lower)
            })
            .ok_or_else(|| anyhow::anyhow!("Browser not found: {browser_name}"))?;

        let browser_type_name = adapter.browser_type().name();

        // Find backup file
        let backup_path = if let Some(file) = backup_file {
            PathBuf::from(file)
        } else {
            // Find latest backup
            let bookmark_path = adapter.detect_bookmark_path()?;
            Self::find_latest_bookmark_backup(&bookmark_path)
                .context("未找到备份文件。请使用 -f 指定备份文件路径")?
        };

        if !backup_path.exists() {
            anyhow::bail!("备份文件不存在: {backup_path:?}");
        }

        info!("📂 备份文件: {:?}", backup_path);

        // Get current bookmark path
        let current_path = adapter.detect_bookmark_path()?;

        // Create a unique backup of current state before restore.
        let pre_restore_backup = adapter
            .backup_bookmarks()
            .context("Refusing to restore without a current-state backup")?;
        info!("💾 当前状态已备份到: {:?}", pre_restore_backup);

        if let Err(error) = restore_bookmark_file(&backup_path, &current_path) {
            return Err(error);
        }

        // Verify
        match adapter.read_bookmarks() {
            Ok(bookmarks) => {
                let count = Self::count_all_bookmarks(&bookmarks);
                info!("✅ 恢复成功! {} 现在有 {} 个书签", browser_type_name, count);
            }
            Err(e) => {
                warn!("⚠️  恢复后验证失败: {}", e);
                warn!("   尝试恢复原状态...");
                let rollback = restore_bookmark_file(&pre_restore_backup, &current_path);
                if let Err(rollback_error) = rollback {
                    anyhow::bail!("恢复失败: {e}; 回滚主库失败: {rollback_error}");
                }
                anyhow::bail!("恢复失败: {e}");
            }
        }

        Ok(())
    }
}

fn validate_master_backup_inventory(inventory: &InventoryReport) -> Result<()> {
    let harvested_profiles: usize = inventory
        .installations
        .iter()
        .map(|installation| installation.profiles.len())
        .sum();

    anyhow::ensure!(
        inventory.total_installations > 0,
        "Backup is incomplete: no browser installations were discovered"
    );
    anyhow::ensure!(
        inventory.total_profiles > 0,
        "Backup is incomplete: discovered installations have no profiles"
    );
    anyhow::ensure!(
        harvested_profiles == inventory.total_profiles,
        "Backup is incomplete: harvested {harvested_profiles} of {} discovered profiles",
        inventory.total_profiles
    );
    anyhow::ensure!(
        inventory.total_bookmarks > 0,
        "Backup is empty: no bookmarks were collected"
    );
    anyhow::ensure!(
        inventory.warnings.is_empty(),
        "Backup is incomplete: {}",
        inventory.warnings.join("; ")
    );
    Ok(())
}

/// Create comprehensive master backup from all browser data
pub async fn create_master_backup(output_dir: &str, include_full: bool) -> Result<()> {
    info!("📦 创建主备份...");

    let output_path = if output_dir.starts_with("~/") {
        let home =
            std::env::var("HOME").map_err(|e| anyhow::anyhow!("获取 HOME 环境变量失败: {e}"))?;
        output_dir.replacen('~', &home, 1)
    } else {
        output_dir.to_string()
    };
    let output_path = PathBuf::from(output_path);
    std::fs::create_dir_all(&output_path)?;

    let (tree, inventory, manifest) = collector::BookmarkCollector::collect_all(&output_path)?;
    validate_master_backup_inventory(&inventory)?;

    fn collect_occurrences(nodes: &[BookmarkNode], result: &mut Vec<serde_json::Value>) {
        for node in nodes {
            match node {
                BookmarkNode::Bookmark(bookmark) => result.push(serde_json::json!({
                    "url": bookmark.raw_url,
                    "title": bookmark.title,
                    "source": bookmark.source.browser.to_string(),
                    "profile_id": bookmark.source.profile_id,
                    "profile_name": bookmark.source.profile_name,
                    "folder_path": bookmark.folder_path,
                })),
                BookmarkNode::Folder(folder) => collect_occurrences(&folder.children, result),
                BookmarkNode::Separator(_) => {}
            }
        }
    }

    let mut all_bookmarks = Vec::new();
    collect_occurrences(&tree.roots, &mut all_bookmarks);

    info!("📊 总计收集: {} 条记录", all_bookmarks.len());

    // Exact URL identity prevents path-case, query, or fragment data loss.
    let mut unique_urls = std::collections::BTreeMap::new();
    for bookmark in &all_bookmarks {
        let url_key = bookmark["url"].as_str().unwrap_or("").trim();
        if !url_key.is_empty() {
            unique_urls
                .entry(url_key.to_string())
                .or_insert(bookmark.clone());
        }
    }

    info!("📊 唯一URL: {} 个", unique_urls.len());

    // 保存文件
    if include_full {
        let full_path = output_path.join("all_bookmarks_full.json");
        crate::browser_utils::atomic_write_file(
            &full_path,
            serde_json::to_string_pretty(&all_bookmarks)?.as_bytes(),
        )?;
        crate::browser_utils::atomic_write_file(
            &output_path.join("bookmark_tree.json"),
            serde_json::to_string_pretty(&tree)?.as_bytes(),
        )?;
        info!("✅ 完整数据: {}", full_path.display());
    }

    let unique_path = output_path.join("unique_bookmarks.json");
    let unique_list: Vec<_> = unique_urls.into_values().collect();
    crate::browser_utils::atomic_write_file(
        &unique_path,
        serde_json::to_string_pretty(&unique_list)?.as_bytes(),
    )?;
    info!("✅ 唯一URL: {}", unique_path.display());

    crate::browser_utils::atomic_write_file(
        &output_path.join("inventory.json"),
        serde_json::to_string_pretty(&inventory)?.as_bytes(),
    )?;
    crate::browser_utils::atomic_write_file(
        &output_path.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;

    let urls_path = output_path.join("all_urls.txt");
    let mut urls: Vec<_> = unique_list
        .iter()
        .filter_map(|b| b["url"].as_str())
        .collect();
    urls.sort_unstable();
    crate::browser_utils::atomic_write_file(&urls_path, urls.join("\n").as_bytes())?;
    info!("✅ URL列表: {}", urls_path.display());

    info!("\n✅ 主备份创建完成: {}", output_path.display());

    Ok(())
}

/// Standalone function: organize bookmarks from multiple local files (no browser engine needed)
///
/// Reads multiple HTML/JSON bookmark files, merges them, deduplicates,
/// classifies using built-in + custom rules, and exports a single organized file.
pub async fn smart_organize_files(
    input_files: &[&str],
    output_file: &str,
    rules_file: Option<&str>,
    classifier: &str,
    ollama_model: &str,
    ollama_url: &str,
    ollama_batch_size: usize,
    deduplicate: bool,
    clean_empty: bool,
    show_stats: bool,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let classify_config = ClassificationConfig {
        rules_file,
        classifier,
        ollama_model,
        ollama_url,
        ollama_batch_size,
    };

    // Read and merge all input files
    let mut bookmarks: Vec<Bookmark> = Vec::new();
    for input_file in input_files {
        let content = std::fs::read_to_string(input_file)
            .context(format!("Failed to read input file: {input_file}"))?;

        let file_bookmarks = if input_file.ends_with(".json") {
            serde_json::from_str::<Vec<Bookmark>>(&content)
                .context(format!("Failed to parse JSON bookmark file: {input_file}"))?
        } else if input_file.ends_with(".html") || input_file.ends_with(".htm") {
            parse_safari_html(&content)?
        } else {
            serde_json::from_str::<Vec<Bookmark>>(&content)
                .or_else(|_| parse_safari_html(&content))
                .context(format!(
                    "Failed to parse bookmark file (tried JSON and HTML): {input_file}"
                ))?
        };

        let count = SyncEngine::count_all_bookmarks(&file_bookmarks);
        info!("📖 Loaded {} bookmarks from: {}", count, input_file);
        bookmarks.extend(file_bookmarks);
    }

    let total_loaded = SyncEngine::count_all_bookmarks(&bookmarks);
    if input_files.len() > 1 {
        info!(
            "📊 Total: {} bookmarks from {} files",
            total_loaded,
            input_files.len()
        );
    }

    // Flatten: remove browser root folders (Brave, Brave Nightly, Waterfox, etc.)
    info!("📦 Flattening: removing browser root folders...");
    let flat_config = FlatExportConfig {
        flatten_root: true,
        root_folders_to_remove: None,
    };
    let (flattened, flatten_stats) = flatten_bookmarks(&bookmarks, &flat_config);
    bookmarks = flattened;
    if flatten_stats.root_folders_removed > 0 {
        info!(
            "  ✅ Removed {} browser root folders, promoted {} items",
            flatten_stats.root_folders_removed, flatten_stats.bookmarks_promoted
        );
    }

    // Deduplicate if requested
    if deduplicate {
        let before = SyncEngine::count_all_bookmarks(&bookmarks);
        SyncEngine::deduplicate_bookmarks_global(&mut bookmarks);
        let after = SyncEngine::count_all_bookmarks(&bookmarks);
        if before > after {
            info!(
                "🔄 Deduplicated: removed {} duplicates ({} → {})",
                before - after,
                before,
                after
            );
        }
    }

    // Clean empty folders if requested
    if clean_empty {
        let removed = SyncEngine::cleanup_empty_folders(&mut bookmarks);
        if removed > 0 {
            info!("🧹 Cleaned {} empty folders", removed);
        }
    }

    // Collect all bookmarks for classification
    let mut to_classify: Vec<Bookmark> = Vec::new();
    SyncEngine::collect_all_bookmarks_for_classification(&mut bookmarks, &mut to_classify);

    info!("  📖 Found {} bookmarks to classify", to_classify.len());
    let (classified, unclassified, stats, backend_used) =
        SyncEngine::classify_bookmarks(&to_classify, &classify_config, verbose).await?;
    info!("  🧠 Backend: {}", backend_used);
    SyncEngine::apply_classification_results(&mut bookmarks, &classified, &unclassified);

    for (folder_name, items) in &classified {
        info!("  📁 {} : {} bookmarks", folder_name, items.len());
    }
    if !unclassified.is_empty() {
        info!("  📁 未分类 : {} bookmarks", unclassified.len());
    }

    // Show statistics
    if show_stats {
        println!("\n📊 Classification Statistics:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Total processed:  {}", stats.total_processed);
        println!(
            "  Total classified: {} ({:.1}%)",
            stats.total_classified,
            if stats.total_processed > 0 {
                stats.total_classified as f64 / stats.total_processed as f64 * 100.0
            } else {
                0.0
            }
        );
        println!("  Unclassified:     {}", stats.unclassified);
        println!("\n  By category:");

        let mut categories: Vec<_> = stats.by_category.iter().collect();
        categories.sort_by(|a, b| b.1.cmp(a.1));
        for (category, count) in categories {
            println!("    📁 {category} : {count}");
        }
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }

    if dry_run {
        info!(
            "  🏃 Dry run - would classify {} bookmarks into {} folders",
            stats.total_classified,
            classified.len()
        );
    } else {
        // Wrap all bookmarks in 📁镜像文件夹 + merge temp folders
        info!("📦 Wrapping all bookmarks in root folder: \"📁镜像文件夹\"");

        // Extract and MERGE all protected folders (👀临时) into one
        let mut merged_temp_children: Vec<Bookmark> = Vec::new();
        bookmarks.retain(|b| {
            if b.folder
                && PROTECTED_FOLDERS
                    .iter()
                    .any(|p| b.title.to_lowercase().contains(&p.to_lowercase()))
            {
                merged_temp_children.extend(b.children.clone());
                false
            } else {
                true
            }
        });

        let wrapped = Bookmark {
            id: "wrap-mirror-folder".to_string(),
            title: "📁镜像文件夹".to_string(),
            url: None,
            folder: true,
            children: bookmarks,
            date_added: Some(chrono::Utc::now().timestamp_millis()),
            date_modified: None,
        };

        let temp_folder = Bookmark {
            id: "temp-folder".to_string(),
            title: "👀临时".to_string(),
            url: None,
            folder: true,
            children: merged_temp_children,
            date_added: Some(chrono::Utc::now().timestamp_millis()),
            date_modified: None,
        };

        bookmarks = vec![wrapped, temp_folder];
        info!("  ✅ Merged all temp folders into single 👀临时");

        // Final clean: remove empty folders again after all processing
        if clean_empty {
            SyncEngine::cleanup_empty_folders(&mut bookmarks);
        }

        // Write output file
        let output_content = if output_file.ends_with(".json") {
            serde_json::to_string_pretty(&bookmarks)?
        } else {
            SyncEngine::export_bookmarks_to_html_internal(&bookmarks)
        };

        crate::browser_utils::atomic_write_file(Path::new(output_file), output_content.as_bytes())
            .context(format!("Failed to write output file: {output_file}"))?;

        let final_count = SyncEngine::count_all_bookmarks(&bookmarks);
        info!(
            "  ✅ Organized {} bookmarks saved to: {}",
            final_count, output_file
        );
    }

    Ok(())
}

/// Export bookmarks to Netscape HTML format (standard bookmark format)
pub fn export_bookmarks_to_html(bookmarks: &[Bookmark], output_path: &str) -> Result<()> {
    let mut output = Vec::new();

    // Write HTML header
    writeln!(output, "<!DOCTYPE NETSCAPE-Bookmark-file-1>")?;
    writeln!(output, "<!-- This is an automatically generated file.")?;
    writeln!(output, "     It will be read and overwritten.")?;
    writeln!(output, "     DO NOT EDIT! -->")?;
    writeln!(
        output,
        "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">"
    )?;
    writeln!(output, "<TITLE>Bookmarks</TITLE>")?;
    writeln!(output, "<H1>Bookmarks</H1>")?;
    writeln!(output, "<DL><p>")?;

    // Write bookmarks recursively
    write_bookmarks_html_recursive(&mut output, bookmarks, 1)?;

    writeln!(output, "</DL><p>")?;

    crate::browser_utils::atomic_write_file(Path::new(output_path), &output)
}

fn write_bookmarks_html_recursive<W: Write>(
    writer: &mut W,
    bookmarks: &[Bookmark],
    indent: usize,
) -> Result<()> {
    let indent_str = "    ".repeat(indent);

    for bookmark in bookmarks {
        if bookmark.folder {
            // Write folder
            let add_date = bookmark.date_added.unwrap_or(0) / 1000; // Convert to seconds
            writeln!(
                writer,
                "{}<DT><H3 ADD_DATE=\"{}\">{}</H3>",
                indent_str,
                add_date,
                html_escape(&bookmark.title)
            )?;
            writeln!(writer, "{indent_str}<DL><p>")?;
            write_bookmarks_html_recursive(writer, &bookmark.children, indent + 1)?;
            writeln!(writer, "{indent_str}</DL><p>")?;
        } else if let Some(url) = &bookmark.url {
            // Write bookmark
            let add_date = bookmark.date_added.unwrap_or(0) / 1000;
            writeln!(
                writer,
                "{}<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>",
                indent_str,
                html_escape(url),
                add_date,
                html_escape(&bookmark.title)
            )?;
        }
    }

    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Import bookmarks from source with detailed JSON reporting
pub fn import_bookmarks_from_source_with_report(
    source_path: &str,
) -> Result<(Vec<Bookmark>, serde_json::Value)> {
    use std::path::Path;
    let path = Path::new(source_path);
    if path.is_dir() {
        let mut all_b = Vec::new();
        let mut db_paths = Vec::new();
        let mut backup_paths = Vec::new();
        let mut skipped_symlink_paths = Vec::new();
        let mut read_errors = Vec::new();
        let mut successful_sources = 0usize;
        for entry in walkdir::WalkDir::new(path).follow_links(false) {
            let entry = entry.map_err(|e| anyhow::anyhow!("扫描目录 '{source_path}' 失败: {e}"))?;
            if entry.file_type().is_symlink() {
                skipped_symlink_paths.push(entry.path().to_string_lossy().into_owned());
                continue;
            }
            if entry.file_name() == "places.sqlite" {
                db_paths.push(entry.path().to_path_buf());
            } else if entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonlz4"))
                && entry
                    .path()
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "bookmarkbackups")
            {
                backup_paths.push(entry.path().to_path_buf());
            }
        }
        db_paths.sort();
        db_paths.dedup();
        backup_paths.sort();
        backup_paths.dedup();

        for db_path in &db_paths {
            info!("🔍 发现 Firefox/Waterfox 数据库文件: {}", db_path.display());
            match crate::browsers::read_firefox_bookmarks(db_path).with_context(|| {
                format!("读取 Firefox/Waterfox 数据库 '{}' 失败", db_path.display())
            }) {
                Ok(b) => {
                    successful_sources += 1;
                    info!("   📚 成功从数据库导入 {} 条书签节点", b.len());
                    all_b.extend(b);
                }
                Err(error) => {
                    read_errors.push(serde_json::json!({
                        "path": db_path,
                        "kind": "sqlite",
                        "error": error.to_string(),
                    }));
                }
            }
        }
        for backup_path in &backup_paths {
            info!(
                "🔍 发现 Firefox/Waterfox JSONLZ4 备份: {}",
                backup_path.display()
            );
            match firefox_backup::import_jsonlz4(backup_path) {
                Ok(b) => {
                    successful_sources += 1;
                    info!("   📚 成功从备份导入 {} 条书签节点", b.len());
                    all_b.extend(b);
                }
                Err(error) => {
                    read_errors.push(serde_json::json!({
                        "path": backup_path,
                        "kind": "jsonlz4",
                        "error": error.to_string(),
                    }));
                }
            }
        }
        if db_paths.is_empty() && backup_paths.is_empty() {
            anyhow::bail!(
                "在目录 '{source_path}' 中未找到 places.sqlite 或 bookmarkbackups/*.jsonlz4"
            );
        }
        if successful_sources == 0 {
            anyhow::bail!(
                "目录 '{}' 中发现的收藏夹数据均无法读取: {}",
                source_path,
                serde_json::to_string(&read_errors).unwrap_or_else(|_| "unknown read error".into())
            );
        }
        let b_count = SyncEngine::count_all_bookmarks(&all_b);
        let f_count = SyncEngine::count_all_folders(&all_b);
        let top_folders: Vec<String> = all_b.iter().map(|b| b.title.clone()).collect();
        let breakdown = SyncEngine::get_folder_breakdown(&all_b);
        let found_db_paths: Vec<String> = db_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let found_backup_paths: Vec<String> = backup_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let report = serde_json::json!({
            "source_path": source_path,
            "source_type": "Firefox/Waterfox Profile Directory",
            "detected_database_paths": found_db_paths,
            "detected_jsonlz4_backup_paths": found_backup_paths,
            "skipped_symlink_paths": skipped_symlink_paths,
            "read_errors": read_errors,
            "status": if read_errors.is_empty() { "ok" } else { "partial" },
            "bookmark_count": b_count,
            "folder_count": f_count,
            "top_level_folders": top_folders,
            "folder_breakdown": breakdown
        });
        Ok((all_b, report))
    } else if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonlz4"))
    {
        let all_b = firefox_backup::import_jsonlz4(path)?;
        let report = serde_json::json!({
            "source_path": source_path,
            "source_type": "Firefox/Waterfox JSONLZ4 Backup",
            "bookmark_count": SyncEngine::count_all_bookmarks(&all_b),
            "folder_count": SyncEngine::count_all_folders(&all_b),
            "top_level_folders": all_b.iter().map(|b| b.title.clone()).collect::<Vec<_>>(),
            "folder_breakdown": SyncEngine::get_folder_breakdown(&all_b)
        });
        Ok((all_b, report))
    } else if path
        .extension()
        .is_some_and(|ext| ext == "sqlite" || ext == "db")
    {
        info!("🔍 读取 SQLite 书签数据库文件: {}", source_path);
        let all_b = crate::browsers::read_firefox_bookmarks(path)
            .with_context(|| format!("读取数据库 '{source_path}' 失败"))?;
        let b_count = SyncEngine::count_all_bookmarks(&all_b);
        let f_count = SyncEngine::count_all_folders(&all_b);
        let top_folders: Vec<String> = all_b.iter().map(|b| b.title.clone()).collect();
        let breakdown = SyncEngine::get_folder_breakdown(&all_b);
        let report = serde_json::json!({
            "source_path": source_path,
            "source_type": "SQLite Database",
            "detected_database_path": source_path,
            "bookmark_count": b_count,
            "folder_count": f_count,
            "top_level_folders": top_folders,
            "folder_breakdown": breakdown
        });
        Ok((all_b, report))
    } else {
        let all_b = import_bookmarks_from_html(source_path)
            .with_context(|| format!("从 HTML 文件 '{source_path}' 导入失败"))?;
        let b_count = SyncEngine::count_all_bookmarks(&all_b);
        let f_count = SyncEngine::count_all_folders(&all_b);
        let top_folders: Vec<String> = all_b.iter().map(|b| b.title.clone()).collect();
        let breakdown = SyncEngine::get_folder_breakdown(&all_b);
        let report = serde_json::json!({
            "source_path": source_path,
            "source_type": "HTML Bookmark File",
            "bookmark_count": b_count,
            "folder_count": f_count,
            "top_level_folders": top_folders,
            "folder_breakdown": breakdown
        });
        Ok((all_b, report))
    }
}

/// Import bookmarks from HTML file, SQLite database, or Firefox/Waterfox profile directory
pub fn import_bookmarks_from_source(source_path: &str) -> Result<Vec<Bookmark>> {
    import_bookmarks_from_source_with_report(source_path).map(|(b, _)| b)
}

/// Import bookmarks from HTML file
pub fn import_bookmarks_from_html(html_path: &str) -> Result<Vec<Bookmark>> {
    let content = std::fs::read_to_string(html_path)?;
    parse_html_bookmarks(&content).with_context(|| format!("parse bookmark HTML {html_path}"))
}

fn parse_html_bookmarks(html: &str) -> Result<Vec<Bookmark>> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let links = Selector::parse("a[href]").map_err(|e| anyhow::anyhow!("HTML selector: {e:?}"))?;
    let lists = Selector::parse("dl").map_err(|e| anyhow::anyhow!("HTML selector: {e:?}"))?;
    let entries =
        Selector::parse("h3, a[href]").map_err(|e| anyhow::anyhow!("HTML selector: {e:?}"))?;
    let expected_urls: Vec<String> = document
        .select(&links)
        .filter_map(|link| link.value().attr("href").map(str::to_owned))
        .collect();
    let expected_titles: Vec<String> = document
        .select(&entries)
        .map(|entry| {
            let text: String = entry.text().collect();
            if entry.value().name() == "h3" {
                text.trim().to_owned()
            } else {
                text
            }
        })
        .collect();
    anyhow::ensure!(
        !expected_urls.is_empty() || document.select(&lists).next().is_some(),
        "not a bookmark HTML export: no bookmark links or folder list found"
    );

    // 使用栈来追踪文件夹层级，保留完整的文件夹结构
    let mut root_bookmarks: Vec<Bookmark> = Vec::new();
    let mut folder_stack: Vec<Bookmark> = Vec::new();
    let mut id_counter = 0u64;
    let mut bookmark_count = 0usize;
    let mut folder_count = 0usize;

    // 预处理：将多行合并，处理跨行的标签
    let lines: Vec<&str> = html.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // 检测文件夹开始: <DT><H3...>FolderName</H3>
        if trimmed.contains("<DT><H3") || trimmed.contains("<dt><h3") {
            if let Some(folder_title) = extract_folder_title(trimmed) {
                id_counter += 1;
                folder_count += 1;
                let add_date = extract_add_date(trimmed);
                let folder = Bookmark {
                    id: format!("folder-{id_counter}"),
                    title: folder_title,
                    url: None,
                    folder: true,
                    children: Vec::new(),
                    date_added: add_date,
                    date_modified: None,
                };
                folder_stack.push(folder);
            }
        }
        // 检测文件夹内容结束: </DL>
        else if trimmed.to_lowercase().starts_with("</dl>") {
            if let Some(completed_folder) = folder_stack.pop() {
                // 将完成的文件夹添加到父级或根级
                if let Some(parent) = folder_stack.last_mut() {
                    parent.children.push(completed_folder);
                } else {
                    root_bookmarks.push(completed_folder);
                }
            }
        }
        // 检测书签: <DT><A HREF="...">Title</A>
        else if (trimmed.contains("<DT><A") || trimmed.contains("<dt><a"))
            && (trimmed.contains("HREF=") || trimmed.contains("href="))
        {
            if let Some((url, title)) = extract_bookmark_info(trimmed) {
                id_counter += 1;
                bookmark_count += 1;
                let add_date = extract_add_date(trimmed);
                let bookmark = Bookmark {
                    id: format!("imported-{id_counter}"),
                    title,
                    url: Some(url),
                    folder: false,
                    children: Vec::new(),
                    date_added: add_date,
                    date_modified: None,
                };
                // 添加到当前文件夹或根级
                if let Some(current_folder) = folder_stack.last_mut() {
                    current_folder.children.push(bookmark);
                } else {
                    root_bookmarks.push(bookmark);
                }
            }
        }

        i += 1;
    }

    // 处理未闭合的文件夹（容错）
    while let Some(unclosed_folder) = folder_stack.pop() {
        warn!("⚠️  未闭合的文件夹: {}", unclosed_folder.title);
        if let Some(parent) = folder_stack.last_mut() {
            parent.children.push(unclosed_folder);
        } else {
            root_bookmarks.push(unclosed_folder);
        }
    }

    let mut pending: Vec<_> = root_bookmarks.iter().rev().collect();
    let mut parsed_titles = Vec::new();
    while let Some(bookmark) = pending.pop() {
        parsed_titles.push(bookmark.title.as_str());
        pending.extend(bookmark.children.iter().rev());
    }
    // The legacy line parser must not silently discard or alter entries accepted by HTML parsing.
    anyhow::ensure!(
        crate::url_checker::collect_urls_from_bookmarks(&root_bookmarks) == expected_urls
            && parsed_titles == expected_titles,
        "bookmark HTML could not be parsed completely without data loss; re-export it in Netscape bookmark format before continuing"
    );
    info!(
        "📖 HTML解析完成: {} 书签, {} 文件夹 (保留结构)",
        bookmark_count, folder_count
    );
    Ok(root_bookmarks)
}

/// 从H3标签提取文件夹标题
fn extract_folder_title(line: &str) -> Option<String> {
    // 查找 <H3...> 和 </H3> 之间的内容
    let line_lower = line.to_ascii_lowercase();
    let h3_start = line_lower.find("<h3")?;
    let content_start = line[h3_start..].find('>')? + h3_start + 1;
    let h3_end = line_lower.find("</h3>")?;

    if content_start < h3_end {
        let title = html_unescape(&line[content_start..h3_end]);
        Some(title.trim().to_string())
    } else {
        None
    }
}

/// `从标签提取ADD_DATE属性`
fn extract_add_date(line: &str) -> Option<i64> {
    let line_lower = line.to_ascii_lowercase();
    if let Some(pos) = line_lower.find("add_date=\"") {
        let start = pos + 10;
        if let Some(end_offset) = line[start..].find('"') {
            if let Ok(timestamp) = line[start..start + end_offset].parse::<i64>() {
                // HTML格式是秒，转换为毫秒
                return timestamp.checked_mul(1000);
            }
        }
    }
    Some(chrono::Utc::now().timestamp_millis())
}

#[must_use]
pub fn extract_tag_content(line: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}");
    let end_tag = format!("</{tag}>");

    if let Some(start_idx) = line
        .to_ascii_lowercase()
        .find(&start_tag.to_ascii_lowercase())
    {
        if let Some(content_start) = line[start_idx..].find('>') {
            let content_start = start_idx + content_start + 1;
            if let Some(end_idx) = line
                .to_ascii_lowercase()
                .find(&end_tag.to_ascii_lowercase())
            {
                let content = &line[content_start..end_idx];
                return Some(html_unescape(content));
            }
        }
    }
    None
}

fn extract_bookmark_info(line: &str) -> Option<(String, String)> {
    // Extract URL from HREF="..."
    let href_start = line.to_ascii_lowercase().find("href=\"")?;
    let url_start = href_start + 6;
    let url_end = line[url_start..].find('"')? + url_start;
    let url = html_unescape(&line[url_start..url_end]);

    // Extract title (content between > and </A>)
    let title_start = line[url_end..].find('>')? + url_end + 1;
    let title_end = line.to_ascii_lowercase().find("</a>")?;
    let title = if title_start < title_end {
        html_unescape(&line[title_start..title_end])
    } else {
        url.clone()
    };

    Some((url, title))
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

// ============================================================================
// Export Quality Improvement - Flat Export, Dedupe, Clean Empty
// ============================================================================

/// 浏览器根文件夹名称列表（用于扁平导出时移除）
const BROWSER_ROOT_FOLDERS: &[&str] = &[
    "waterfox",
    "brave",
    "brave nightly",
    "chrome",
    "safari",
    "edge",
    "firefox",
    "opera",
    "vivaldi",
    "arc",
    "chromium",
    "书签栏",
    "bookmarks bar",
    "bookmark bar",
    "toolbar",
    "其他书签",
    "other bookmarks",
    "other",
    "移动设备书签",
    "mobile bookmarks",
    "imported", // 浏览器导入时自动创建的文件夹
];

/// 受保护的文件夹名称（即使为空也保留）
const PROTECTED_FOLDERS: &[&str] = &["👀临时", "👀 临时", "临时", "temp", "temporary"];

/// 扁平导出配置
#[derive(Debug, Clone, Default)]
pub struct FlatExportConfig {
    /// 是否移除浏览器根文件夹
    pub flatten_root: bool,
    /// 要移除的根文件夹名称列表（可选，默认使用内置列表）
    pub root_folders_to_remove: Option<Vec<String>>,
}

/// 扁平导出统计
#[derive(Debug, Clone, Default)]
pub struct FlattenStats {
    pub root_folders_removed: usize,
    pub bookmarks_promoted: usize,
}

/// 扁平化书签结构，移除浏览器特定根文件夹（原地修改版本，避免内存爆炸）
/// 将 Waterfox/Brave/Chrome 等根文件夹的内容提升到顶层
pub fn flatten_bookmarks(
    bookmarks: &[Bookmark],
    config: &FlatExportConfig,
) -> (Vec<Bookmark>, FlattenStats) {
    let mut stats = FlattenStats::default();

    if !config.flatten_root {
        return (bookmarks.to_vec(), stats);
    }

    let root_folders: Vec<String> = config.root_folders_to_remove.clone().unwrap_or_else(|| {
        BROWSER_ROOT_FOLDERS
            .iter()
            .map(ToString::to_string)
            .collect()
    });

    // 原地修改：只在顶层移除浏览器根文件夹
    flatten_bookmarks_inplace_impl(bookmarks, &root_folders, &mut stats)
}

/// 递归扁平化实现 - 处理所有层级的浏览器根文件夹
fn flatten_bookmarks_inplace_impl(
    bookmarks: &[Bookmark],
    root_folders: &[String],
    stats: &mut FlattenStats,
) -> (Vec<Bookmark>, FlattenStats) {
    let mut result = Vec::with_capacity(bookmarks.len());

    for bookmark in bookmarks {
        if bookmark.folder {
            let title_lower = bookmark.title.to_lowercase();
            // 检查是否是浏览器根文件夹
            let is_browser_root = root_folders.iter().any(|rf| {
                let rf_lower = rf.to_lowercase();
                title_lower == rf_lower || title_lower.contains(&rf_lower)
            });

            if is_browser_root {
                // 移除根文件夹，将其子内容提升（递归处理子内容中的浏览器文件夹）
                stats.root_folders_removed += 1;
                stats.bookmarks_promoted += bookmark.children.len();
                // 递归处理子内容，继续移除嵌套的浏览器根文件夹
                let (flattened_children, _) =
                    flatten_bookmarks_inplace_impl(&bookmark.children, root_folders, stats);
                result.extend(flattened_children);
            } else {
                // 保留文件夹，但递归处理其子内容
                let mut new_bookmark = bookmark.clone();
                let (flattened_children, _) =
                    flatten_bookmarks_inplace_impl(&bookmark.children, root_folders, stats);
                new_bookmark.children = flattened_children;
                result.push(new_bookmark);
            }
        } else {
            // 非文件夹直接保留
            result.push(bookmark.clone());
        }
    }

    (result, stats.clone())
}

/// 去重统计
#[derive(Debug, Clone, Default)]
pub struct DedupeStats {
    pub total_before: usize,
    pub duplicates_removed: usize,
    pub total_after: usize,
}

/// 去除重复书签（基于 URL），保留第一个出现的
pub fn deduplicate_bookmarks(bookmarks: &mut Vec<Bookmark>) -> DedupeStats {
    let mut stats = DedupeStats::default();
    let mut seen_urls: HashSet<String> = HashSet::new();

    fn count_bookmarks(bookmarks: &[Bookmark]) -> usize {
        bookmarks
            .iter()
            .map(|b| {
                if b.folder {
                    count_bookmarks(&b.children)
                } else {
                    1
                }
            })
            .sum()
    }

    stats.total_before = count_bookmarks(bookmarks);

    fn dedupe_recursive(bookmarks: &mut Vec<Bookmark>, seen: &mut HashSet<String>) -> usize {
        let mut removed = 0;

        // 先递归处理子文件夹
        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder {
                removed += dedupe_recursive(&mut bookmark.children, seen);
            }
        }

        // 然后处理当前层级
        bookmarks.retain(|b| {
            if b.folder {
                true // 保留文件夹
            } else if let Some(ref url) = b.url {
                let normalized = normalize_url_for_dedupe(url);
                if seen.contains(&normalized) {
                    removed += 1;
                    false // 移除重复
                } else {
                    seen.insert(normalized);
                    true
                }
            } else {
                true // 保留无 URL 的书签
            }
        });

        removed
    }

    stats.duplicates_removed = dedupe_recursive(bookmarks, &mut seen_urls);
    stats.total_after = count_bookmarks(bookmarks);
    stats
}

/// Return the byte-exact URL key used by the cleanup workflow.
///
/// Case, fragments, trailing slashes, whitespace, and `www` can all identify
/// distinct bookmark entries.  Collapsing any of them makes precise cleanup
/// impossible to audit.
fn normalize_url_for_dedupe(url: &str) -> String {
    url.to_string()
}

/// 清理统计
#[derive(Debug, Clone, Default)]
pub struct CleanStats {
    pub empty_folders_removed: usize,
}

/// 检查文件夹是否受保护（即使为空也保留）
fn is_protected_folder(title: &str) -> bool {
    let title_lower = title.to_lowercase();
    PROTECTED_FOLDERS.iter().any(|pf| {
        let pf_lower = pf.to_lowercase();
        title_lower == pf_lower || title_lower.contains(&pf_lower)
    })
}

/// 递归移除空文件夹（保护特定文件夹）
pub fn clean_empty_folders(bookmarks: &mut Vec<Bookmark>) -> CleanStats {
    let mut stats = CleanStats::default();

    fn clean_recursive(bookmarks: &mut Vec<Bookmark>) -> usize {
        let mut removed = 0;

        // 先递归处理子文件夹
        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder {
                removed += clean_recursive(&mut bookmark.children);
            }
        }

        // 然后移除空文件夹（但保护特定文件夹）
        let before_len = bookmarks.len();
        bookmarks.retain(|b| {
            if b.folder && b.children.is_empty() {
                // 检查是否是受保护的文件夹
                if is_protected_folder(&b.title) {
                    true // 保留受保护的空文件夹
                } else {
                    false // 移除普通空文件夹
                }
            } else {
                true
            }
        });
        removed += before_len - bookmarks.len();

        removed
    }

    stats.empty_folders_removed = clean_recursive(bookmarks);
    stats
}

/// 确保受保护的文件夹存在（如 👀临时）
pub fn ensure_protected_folders(bookmarks: &mut Vec<Bookmark>) {
    // 检查 👀临时 文件夹是否存在
    let temp_folder_exists = bookmarks
        .iter()
        .any(|b| b.folder && is_protected_folder(&b.title));

    if !temp_folder_exists {
        // 添加 👀临时 文件夹到顶层
        let temp_folder = Bookmark {
            id: "protected-temp-folder".to_string(),
            title: "👀临时".to_string(),
            url: None,
            folder: true,
            children: vec![],
            date_added: Some(chrono::Utc::now().timestamp_millis()),
            date_modified: None,
        };
        bookmarks.push(temp_folder);
        info!("  ✅ Added protected folder: 👀临时");
    }
}

/// 更新统计
#[derive(Debug, Clone, Default)]
pub struct UpdateStats {
    pub new_added: usize,
    pub skipped_duplicates: usize,
}

/// 将新书签合并到现有书签（增量更新）
pub fn merge_bookmarks_incremental(
    existing: &mut Vec<Bookmark>,
    new_bookmarks: &[Bookmark],
) -> UpdateStats {
    let mut stats = UpdateStats::default();

    // 收集现有的所有 URL
    fn collect_urls(bookmarks: &[Bookmark], urls: &mut HashSet<String>) {
        for b in bookmarks {
            if b.folder {
                collect_urls(&b.children, urls);
            } else if let Some(ref url) = b.url {
                urls.insert(normalize_url_for_dedupe(url));
            }
        }
    }

    let mut existing_urls = HashSet::new();
    collect_urls(existing, &mut existing_urls);

    // 添加新书签（跳过已存在的）
    fn add_new_bookmarks(
        target: &mut Vec<Bookmark>,
        source: &[Bookmark],
        existing_urls: &HashSet<String>,
        stats: &mut UpdateStats,
    ) {
        for bookmark in source {
            if bookmark.folder {
                // 查找或创建同名文件夹
                let existing_folder = target
                    .iter_mut()
                    .find(|b| b.folder && b.title.to_lowercase() == bookmark.title.to_lowercase());

                if let Some(folder) = existing_folder {
                    // 递归合并到现有文件夹
                    add_new_bookmarks(
                        &mut folder.children,
                        &bookmark.children,
                        existing_urls,
                        stats,
                    );
                } else {
                    // 创建新文件夹并添加内容
                    let mut new_folder = bookmark.clone();
                    new_folder.children = Vec::new();
                    add_new_bookmarks(
                        &mut new_folder.children,
                        &bookmark.children,
                        existing_urls,
                        stats,
                    );
                    if !new_folder.children.is_empty() || bookmark.children.is_empty() {
                        target.push(new_folder);
                    }
                }
            } else if let Some(ref url) = bookmark.url {
                let normalized = normalize_url_for_dedupe(url);
                if existing_urls.contains(&normalized) {
                    stats.skipped_duplicates += 1;
                } else {
                    target.push(bookmark.clone());
                    stats.new_added += 1;
                }
            }
        }
    }

    add_new_bookmarks(existing, new_bookmarks, &existing_urls, &mut stats);
    stats
}

/// 带扁平导出选项的 HTML 导出
pub fn export_bookmarks_to_html_flat(
    bookmarks: &[Bookmark],
    output_path: &str,
    flat_config: Option<&FlatExportConfig>,
    dedupe: bool,
    clean_empty: bool,
) -> Result<(usize, FlattenStats, DedupeStats, CleanStats)> {
    let mut working_bookmarks = bookmarks.to_vec();
    let mut flatten_stats = FlattenStats::default();
    let mut dedupe_stats = DedupeStats::default();
    let mut clean_stats = CleanStats::default();

    // 1. 扁平化（如果配置了）
    if let Some(config) = flat_config {
        let (flattened, stats) = flatten_bookmarks(&working_bookmarks, config);
        working_bookmarks = flattened;
        flatten_stats = stats;
        if flatten_stats.root_folders_removed > 0 {
            info!(
                "📦 扁平化: 移除 {} 个浏览器根文件夹，提升 {} 个项目",
                flatten_stats.root_folders_removed, flatten_stats.bookmarks_promoted
            );
        }
    }

    // 2. 去重（如果启用）
    if dedupe {
        dedupe_stats = deduplicate_bookmarks(&mut working_bookmarks);
        if dedupe_stats.duplicates_removed > 0 {
            info!(
                "🔄 去重: 移除 {} 个重复书签 ({} → {})",
                dedupe_stats.duplicates_removed,
                dedupe_stats.total_before,
                dedupe_stats.total_after
            );
        }
    }

    // 3. 清理空文件夹（如果启用）
    if clean_empty {
        clean_stats = clean_empty_folders(&mut working_bookmarks);
        if clean_stats.empty_folders_removed > 0 {
            info!(
                "🧹 清理: 移除 {} 个空文件夹",
                clean_stats.empty_folders_removed
            );
        }
    }

    // 4. 导出
    export_bookmarks_to_html(&working_bookmarks, output_path)?;

    // 计算最终书签数
    fn count_final(bookmarks: &[Bookmark]) -> usize {
        bookmarks
            .iter()
            .map(|b| {
                if b.folder {
                    count_final(&b.children)
                } else {
                    1
                }
            })
            .sum()
    }
    let final_count = count_final(&working_bookmarks);

    Ok((final_count, flatten_stats, dedupe_stats, clean_stats))
}

/// Merge check result files (valid.html, invalid.html, uncertain.html, skipped.html) from `result_dir` into `output_path`
pub fn merge_check_result_files(result_dir: &str, output_path: &str) -> Result<usize> {
    use std::path::Path;

    let dir_path = Path::new(result_dir);
    anyhow::ensure!(
        dir_path.is_dir(),
        "Result directory does not exist or is not a directory: {result_dir}"
    );

    info!("📦 Merging check results from: {}", result_dir);

    let mut result_folders = Vec::new();

    let sections = [
        ("valid.html", "✅ 有效"),
        ("invalid.html", "❌ 无效"),
        ("uncertain.html", "❓ 不确定"),
        ("skipped.html", "⏭️ 跳过"),
    ];

    let resolved_dir = std::fs::canonicalize(dir_path)?;
    let output = Path::new(output_path);
    let resolved_output = if output.try_exists()? {
        std::fs::canonicalize(output)?
    } else {
        let parent = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::canonicalize(parent)?.join(output.file_name().context("output has no filename")?)
    };
    for (file_name, _) in &sections {
        let source = resolved_dir.join(file_name);
        let resolved_source = if source.try_exists()? {
            std::fs::canonicalize(&source)?
        } else {
            source
        };
        anyhow::ensure!(
            resolved_output != resolved_source,
            "merge output must not overwrite result partition {file_name}"
        );
    }

    for (file_name, section_title) in &sections {
        let file_path = dir_path.join(file_name);
        let partition_present = match std::fs::symlink_metadata(&file_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).with_context(|| format!("inspect {}", file_path.display()))
            }
        };
        if partition_present {
            let bookmarks =
                import_bookmarks_from_html(&file_path.to_string_lossy()).with_context(|| {
                    format!("Failed to read result partition {}", file_path.display())
                })?;
            // Strip the top-level 📁镜像文件夹 wrapper if present,
            // extracting its children directly — equivalent to shell's extract_inner().
            let inner: Vec<Bookmark> = if bookmarks.len() == 1
                && bookmarks[0].folder
                && bookmarks[0].title.contains("镜像文件夹")
            {
                let mut bm = bookmarks;
                let wrapper = bm.remove(0);
                info!(
                    "显式解包顶层镜像文件夹 '{}'，包含 {} 个子书签",
                    wrapper.title,
                    wrapper.children.len()
                );
                wrapper.children
            } else {
                bookmarks
            };

            let count = count_tree_bookmarks(&inner);
            info!("   {}: {} bookmarks", section_title, count);
            let folder = Bookmark {
                id: format!("section-{}", file_name.replace(".html", "")),
                title: section_title.to_string(),
                url: None,
                folder: true,
                children: inner,
                date_added: Some(chrono::Utc::now().timestamp_millis()),
                date_modified: None,
            };
            result_folders.push(folder);
        }
    }
    anyhow::ensure!(
        !result_folders.is_empty(),
        "No check-result partitions found in {result_dir}"
    );

    // Add temp placeholder folder
    let temp_folder = Bookmark {
        id: "section-temp".to_string(),
        title: "👀 临时".to_string(),
        url: None,
        folder: true,
        children: vec![Bookmark {
            id: "placeholder-1".to_string(),
            title: "📌 临时书签放这里".to_string(),
            url: Some("about:blank".to_string()),
            folder: false,
            children: vec![],
            date_added: Some(chrono::Utc::now().timestamp_millis()),
            date_modified: None,
        }],
        date_added: Some(chrono::Utc::now().timestamp_millis()),
        date_modified: None,
    };
    result_folders.push(temp_folder);

    // Wrap in top-level 📁镜像文件夹
    let mirror_wrap = Bookmark {
        id: "mirror-wrap-root".to_string(),
        title: "📁镜像文件夹".to_string(),
        url: None,
        folder: true,
        children: result_folders,
        date_added: Some(chrono::Utc::now().timestamp_millis()),
        date_modified: None,
    };

    let final_bookmarks = vec![mirror_wrap];
    let count = count_tree_bookmarks(&final_bookmarks);

    export_bookmarks_to_html(&final_bookmarks, output_path)?;
    info!("✅ Merged {} bookmarks to {}", count, output_path);
    Ok(count)
}

fn count_tree_bookmarks(bookmarks: &[Bookmark]) -> usize {
    let mut count = 0;
    for b in bookmarks {
        if b.url.is_some() {
            count += 1;
        }
        count += count_tree_bookmarks(&b.children);
    }
    count
}

// ============================================================================
// Property-Based Tests for Export Quality
// ============================================================================

#[cfg(test)]
mod export_quality_tests {
    use super::*;
    use proptest::prelude::*;

    // Helper: create a bookmark
    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: format!("id-{title}"),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: Some(1000000),
            date_modified: None,
        }
    }

    // Helper: create a folder
    fn make_folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: format!("folder-{title}"),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: Some(1000000),
            date_modified: None,
        }
    }

    // Strategy: generate random bookmark titles (including Unicode)
    fn arb_title() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-zA-Z0-9_\\-\\u4e00-\\u9fff]{1,20}")
            .unwrap()
            .prop_filter("non-empty", |s| !s.is_empty())
    }

    // Strategy: generate random URLs
    fn arb_url() -> impl Strategy<Value = String> {
        prop::string::string_regex("https?://[a-z0-9]+\\.[a-z]{2,4}/[a-z0-9]*").unwrap()
    }

    // **Feature: export-quality-improvement, Property 3: 扁平导出无根文件夹**
    // **Validates: Requirements 2.1, 2.2**
    proptest! {
        #[test]
        fn prop_flatten_removes_browser_roots(
            user_folder in arb_title(),
            bookmark_title in arb_title(),
            url in arb_url()
        ) {
            // Create structure: Waterfox > user_folder > bookmark
            let bookmark = make_bookmark(&bookmark_title, &url);
            let user_folder_node = make_folder(&user_folder, vec![bookmark]);
            let waterfox_root = make_folder("Waterfox", vec![user_folder_node]);
            let bookmarks = vec![waterfox_root];

            let config = FlatExportConfig {
                flatten_root: true,
                root_folders_to_remove: None,
            };
            let (result, stats) = flatten_bookmarks(&bookmarks, &config);

            // Property: no browser root folders in result
            for b in &result {
                let title_lower = b.title.to_lowercase();
                prop_assert!(!title_lower.contains("waterfox"));
                prop_assert!(!title_lower.contains("brave"));
                prop_assert!(!title_lower.contains("chrome"));
            }
            // At least one root was removed
            prop_assert!(stats.root_folders_removed >= 1);
        }
    }

    #[test]
    fn exact_dedupe_keeps_url_variants_distinct() {
        let mut bookmarks = vec![
            make_bookmark("plain", "https://example.com/path"),
            make_bookmark("slash", "https://example.com/path/"),
            make_bookmark("fragment", "https://example.com/path#section"),
            make_bookmark("case", "https://EXAMPLE.com/path"),
            make_bookmark("exact duplicate", "https://example.com/path"),
        ];

        let stats = deduplicate_bookmarks(&mut bookmarks);

        assert_eq!(stats.duplicates_removed, 1);
        assert_eq!(bookmarks.len(), 4);
    }

    // **Feature: export-quality-improvement, Property 6: 去重正确性**
    // **Validates: Requirements 5.1, 5.2**
    proptest! {
        #[test]
        fn prop_dedupe_no_duplicate_urls(
            title1 in arb_title(),
            title2 in arb_title(),
            url in arb_url()
        ) {
            // Create two bookmarks with same URL
            let b1 = make_bookmark(&title1, &url);
            let b2 = make_bookmark(&title2, &url);
            let mut bookmarks = vec![b1, b2];

            let stats = deduplicate_bookmarks(&mut bookmarks);

            // Property: no duplicate URLs after deduplication
            let urls: Vec<_> = bookmarks.iter()
                .filter_map(|b| b.url.as_ref())
                .map(|u| normalize_url_for_dedupe(u))
                .collect();
            let unique: HashSet<_> = urls.iter().collect();
            prop_assert_eq!(urls.len(), unique.len());

            // First bookmark should be kept
            prop_assert_eq!(bookmarks.len(), 1);
            prop_assert!(stats.duplicates_removed >= 1);
        }
    }

    // **Feature: export-quality-improvement, Property 5: 空文件夹清理**
    // **Validates: Requirements 4.1, 4.2**
    proptest! {
        #[test]
        fn prop_clean_no_empty_folders(
            folder_name in arb_title()
        ) {
            // Create empty folder
            let empty_folder = make_folder(&folder_name, vec![]);
            let mut bookmarks = vec![empty_folder];

            let stats = clean_empty_folders(&mut bookmarks);

            // Property: no empty folders after cleaning
            fn has_empty_folder(bookmarks: &[Bookmark]) -> bool {
                for b in bookmarks {
                    if b.folder && b.children.is_empty() {
                        return true;
                    }
                    if has_empty_folder(&b.children) {
                        return true;
                    }
                }
                false
            }
            prop_assert!(!has_empty_folder(&bookmarks));
            prop_assert!(stats.empty_folders_removed >= 1);
        }
    }

    // **Feature: export-quality-improvement, Property 1: 文件夹名称保真度**
    // **Validates: Requirements 1.1, 1.2**
    proptest! {
        #[test]
        fn prop_folder_name_preserved(
            folder_name in arb_title()
        ) {
            let folder = make_folder(&folder_name, vec![make_bookmark("test", "http://test.com")]);
            let bookmarks = vec![folder];

            // Export to HTML string
            let html = SyncEngine::export_bookmarks_to_html_internal(&bookmarks);

            // Property: folder name appears in HTML (escaped)
            let escaped_name = html_escape(&folder_name);
            prop_assert!(html.contains(&escaped_name),
                "Folder name '{}' (escaped: '{}') not found in HTML", folder_name, escaped_name);
        }
    }

    // **Feature: export-quality-improvement, Property 2: HTML 特殊字符转义**
    // **Validates: Requirements 1.3**
    #[test]
    fn test_html_escape_special_chars() {
        // Test specific special characters
        let test_cases = vec![
            ("Test & More", "Test &amp; More"),
            ("A < B", "A &lt; B"),
            ("A > B", "A &gt; B"),
            ("Say \"Hello\"", "Say &quot;Hello&quot;"),
        ];

        for (input, expected) in test_cases {
            let result = html_escape(input);
            assert_eq!(result, expected, "Failed for input: {input}");
        }
    }

    // **Feature: export-quality-improvement, Property 7: 增量更新正确性**
    // **Validates: Requirements 6.1, 6.2**
    proptest! {
        #[test]
        fn prop_incremental_update_no_duplicates(
            title in arb_title(),
            url in arb_url()
        ) {
            // Existing bookmark
            let existing = make_bookmark(&title, &url);
            let mut existing_bookmarks = vec![existing];

            // New bookmark with same URL
            let new_bookmark = make_bookmark("new_title", &url);
            let new_bookmarks = vec![new_bookmark];

            let stats = merge_bookmarks_incremental(&mut existing_bookmarks, &new_bookmarks);

            // Property: duplicate URL should be skipped
            prop_assert_eq!(stats.skipped_duplicates, 1);
            prop_assert_eq!(stats.new_added, 0);
            prop_assert_eq!(existing_bookmarks.len(), 1);
        }
    }

    #[test]
    fn builtin_rules_do_not_contain_exact_duplicates() {
        let mut seen = HashSet::new();
        for rule in get_builtin_rules() {
            let serialized = serde_json::to_string(&rule).unwrap();
            assert!(
                seen.insert(serialized),
                "duplicate built-in rule: {}",
                rule.name
            );
        }
    }

    #[test]
    fn unreadable_external_profile_is_reported_instead_of_silently_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let backup_dir = temp.path().join("bookmarkbackups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(backup_dir.join("bad.jsonlz4"), b"not-jsonlz4").unwrap();

        let error = import_bookmarks_from_source_with_report(temp.path().to_str().unwrap())
            .expect_err("a directory whose only source is unreadable must fail closed");
        assert!(error.to_string().contains("均无法读取"));
    }
}
