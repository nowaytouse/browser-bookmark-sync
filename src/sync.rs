// Allow dead code for reserved/future features (sync, reading list, cookies, migration)
#![allow(dead_code)]

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::io::Write;
use tracing::{info, warn, error, debug};
use sha2::{Sha256, Digest};

use crate::browsers::{Bookmark, BrowserAdapter, BrowserType, get_all_adapters, HistoryItem, ReadingListItem, Cookie};
use crate::validator::ValidationReport;

/// Location information for a bookmark in the tree
struct BookmarkLocation {
    path: BookmarkPath,  // Vector of indices representing the path in the tree
    depth: usize,
    date_added: Option<i64>,
}

/// Path to a bookmark in the tree (sequence of indices)
type BookmarkPath = Vec<usize>;

/// Sync mode (reserved for future incremental sync feature)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SyncMode {
    /// Incremental sync - only sync changes since last sync
    Incremental,
}

/// Sync statistics (reserved for future sync feature)
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct SyncStats {
    pub bookmarks_synced: usize,
    pub duplicates_removed: usize,
    pub conflicts_resolved: usize,
    pub errors: usize,
}

pub struct SyncEngine {
    adapters: Vec<Box<dyn BrowserAdapter + Send + Sync>>,
    #[allow(dead_code)]
    last_sync_time: Option<i64>,
}

impl SyncEngine {
    pub fn new() -> Result<Self> {
        let adapters = get_all_adapters();
        Ok(Self { 
            adapters,
            last_sync_time: None,
        })
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
                        debug!("Read {} history items from {}", history.len(), adapter.browser_type().name());
                        all_history.append(&mut history);
                    }
                    Err(e) => {
                        warn!("Failed to read history from {}: {}", adapter.browser_type().name(), e);
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
                        debug!("Read {} cookies from {}", cookies.len(), adapter.browser_type().name());
                        all_cookies.append(&mut cookies);
                    }
                    Err(e) => {
                        warn!("Failed to read cookies from {}: {}", adapter.browser_type().name(), e);
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
        std::fs::write(&state_file, timestamp.to_string())?;
        debug!("Saved sync time: {}", timestamp);
        Ok(())
    }
    
    /// Get state file path
    fn get_state_file_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(format!("{}/.browser-sync/last_sync", home)))
    }

    pub async fn sync(&mut self, mode: SyncMode, dry_run: bool, verbose: bool) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        // Load last sync time for incremental mode
        if mode == SyncMode::Incremental {
            let _ = self.load_last_sync_time();
            if let Some(last_sync) = self.last_sync_time {
                info!("🔄 Incremental sync mode (last sync: {})", 
                    chrono::DateTime::from_timestamp_millis(last_sync)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "unknown".to_string())
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
                    warn!("⚠️  Failed to read bookmarks from {}: {}", browser_type.name(), e);
                    stats.errors += 1;
                }
            }
        }

        if browser_bookmarks.is_empty() {
            error!("❌ No bookmarks could be read from any browser");
            anyhow::bail!("No bookmarks available for synchronization");
        }

        info!("🧹 Phase 3: Pre-merge deduplication (smart selection)");
        let before_dedup = browser_bookmarks.values()
            .map(|b| Self::count_all_bookmarks(b))
            .sum::<usize>();
        
        // Smart deduplication for each browser (depth > date > root)
        for (browser_type, bookmarks) in browser_bookmarks.iter_mut() {
            let before = Self::count_all_bookmarks(bookmarks);
            Self::deduplicate_bookmarks_global(bookmarks);
            let after = Self::count_all_bookmarks(bookmarks);
            let removed = before.saturating_sub(after);
            if removed > 0 && verbose {
                debug!("  {} : removed {} duplicates", browser_type.name(), removed);
            }
        }
        
        let after_dedup = browser_bookmarks.values()
            .map(|b| Self::count_all_bookmarks(b))
            .sum::<usize>();
        
        let dedup_count = before_dedup.saturating_sub(after_dedup);
        if dedup_count > 0 {
            info!("🔄 Pre-merge: removed {} duplicates (smart selection)", dedup_count);
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
            info!("🔄 Post-merge: removed {} duplicates (final cleanup)", final_dedup_count);
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
            debug!("📊 Total reduction: {} → {} bookmarks ({:.1}% reduction)", 
                before_dedup, after_final_dedup, reduction_pct);
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
                    info!("✅ Backup created for {}: {:?}", adapter.browser_type().name(), backup_path);
                }
                Err(e) => {
                    warn!("⚠️  Failed to backup {}: {}", adapter.browser_type().name(), e);
                    stats.errors += 1;
                }
            }
        }

        info!("✍️  Phase 7: Writing merged bookmarks");
        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();
            match adapter.write_bookmarks(&merged) {
                Ok(_) => {
                    info!("✅ Wrote bookmarks to {}", browser_type.name());
                }
                Err(e) => {
                    error!("❌ Failed to write bookmarks to {}: {}", browser_type.name(), e);
                    stats.errors += 1;
                }
            }
        }

        info!("🔍 Phase 8: Post-sync validation");
        match self.post_sync_validation(&merged) {
            Ok(_) => {},
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
                    debug!("✅ {} detected at: {:?}", adapter.browser_type().name(), path);
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

        info!("✅ Pre-sync validation passed: {} browsers detected", detected);
        Ok(())
    }

    fn post_sync_validation(&self, expected: &[Bookmark]) -> Result<()> {
        let mut validation_passed = true;
        let expected_count = Self::count_all_bookmarks(expected);
        let expected_folders = Self::count_all_folders(expected);

        info!("🔍 Validating sync results...");
        info!("   Expected: {} bookmarks, {} folders", expected_count, expected_folders);

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
                        warn!("⚠️  {} : bookmark count mismatch (expected: {}, actual: {})", 
                            browser_name, expected_count, actual_count);
                        validation_passed = false;
                    } else if folder_diff > 2 {
                        warn!("⚠️  {} : folder count mismatch (expected: {}, actual: {})", 
                            browser_name, expected_folders, actual_folders);
                        validation_passed = false;
                    } else {
                        debug!("✅ {} : validation passed ({} bookmarks, {} folders)", 
                            browser_name, actual_count, actual_folders);
                    }
                    
                    // Check for duplicates
                    let mut url_set = HashSet::new();
                    let mut duplicate_count = 0;
                    Self::check_duplicates_recursive(&bookmarks, &mut url_set, &mut duplicate_count);
                    
                    if duplicate_count > 0 {
                        warn!("⚠️  {} : found {} duplicate URLs", browser_name, duplicate_count);
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
    fn check_duplicates_recursive(bookmarks: &[Bookmark], url_set: &mut HashSet<String>, duplicate_count: &mut usize) {
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
                debug!("Browser {} has {} bookmarks, {} folders (score: {})", 
                    browser.name(), url_count, folder_count, score);
            }
            
            info!("📊 {} structure: {} URLs, {} folders", browser.name(), url_count, folder_count);
            
            if score > best_score {
                best_score = score;
                best_browser = Some(*browser);
            }
        }
        
        // Use the best browser's bookmarks as base (preserving folder structure)
        let mut merged = if let Some(browser) = best_browser {
            let bookmarks = browser_bookmarks.get(&browser).cloned().unwrap_or_default();
            let url_count = Self::count_all_bookmarks(&bookmarks);
            let folder_count = Self::count_all_folders(&bookmarks);
            info!("📚 Using {} as base ({} URLs, {} folders)", browser.name(), url_count, folder_count);
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
            info!("   Removed {} duplicate URLs ({} → {})", 
                before_count - after_count, before_count, after_count);
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
    fn deduplicate_bookmarks_global(bookmarks: &mut Vec<Bookmark>) {
        // Two-pass strategy:
        // Pass 1: Collect all bookmarks with their metadata
        // Pass 2: For each URL, decide which one to keep, mark others for deletion
        let mut url_map: HashMap<String, Vec<BookmarkLocation>> = HashMap::new();
        Self::collect_all_bookmarks(bookmarks, &mut url_map, 0, &[]);
        
        // Determine which bookmark to keep for each URL
        let mut urls_to_keep: HashMap<String, BookmarkPath> = HashMap::new();
        for (url, locations) in url_map.iter() {
            if locations.len() > 1 {
                // Find the best bookmark according to priority rules
                let best = Self::select_best_bookmark(locations);
                urls_to_keep.insert(url.clone(), best.path.clone());
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
                
                url_map.entry(normalized).or_insert_with(Vec::new).push(location);
            }
        }
    }
    
    /// Select the best bookmark from duplicates
    /// Rules:
    /// 1. Prefer bookmarks in deeper folder structure
    /// 2. If same depth, prefer newer (larger date_added)
    /// 3. If depth=0 for all, prefer newest
    fn select_best_bookmark(locations: &[BookmarkLocation]) -> &BookmarkLocation {
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
        }).unwrap()
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
                Self::remove_duplicates_by_path(&mut bookmark.children, urls_to_keep, &current_path);
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
        let mut normalized = url.trim().to_lowercase();
        // Remove trailing slash
        if normalized.ends_with('/') {
            normalized.pop();
        }
        // Remove fragment
        if let Some(pos) = normalized.find('#') {
            normalized.truncate(pos);
        }
        normalized
    }
    
    fn count_all_folders(bookmarks: &[Bookmark]) -> usize {
        let mut count = 0;
        for b in bookmarks {
            if b.folder {
                count += 1;
                count += Self::count_all_folders(&b.children);
            }
        }
        count
    }
    
    fn count_all_bookmarks(bookmarks: &[Bookmark]) -> usize {
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
    
    /// Remove empty folders and folders with invalid names
    fn cleanup_empty_folders(bookmarks: &mut Vec<Bookmark>) -> usize {
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
    
    /// Deduplicate folder structures by signature
    fn deduplicate_folder_structures(bookmarks: &mut Vec<Bookmark>) -> usize {
        let mut removed_count = 0;
        
        fn get_folder_signature(bookmark: &Bookmark) -> String {
            if !bookmark.folder {
                return String::new();
            }
            
            // Signature: name + child count + first 3 child names
            let child_names: Vec<String> = bookmark.children.iter()
                .take(3)
                .map(|c| c.title.clone())
                .collect();
            
            format!("{}|{}|{}", bookmark.title, bookmark.children.len(), child_names.join(","))
        }
        
        fn deduplicate_recursive(bookmarks: &mut Vec<Bookmark>, removed: &mut usize) {
            // Build signature map
            let mut seen_signatures: HashMap<String, usize> = HashMap::new();
            let mut to_remove: Vec<usize> = Vec::new();
            
            for (idx, bookmark) in bookmarks.iter().enumerate() {
                if bookmark.folder {
                    let signature = get_folder_signature(bookmark);
                    if !signature.is_empty() {
                        if seen_signatures.contains_key(&signature) {
                            // Duplicate found
                            debug!("Found duplicate folder: {} (signature: {})", bookmark.title, signature);
                            to_remove.push(idx);
                            *removed += 1;
                        } else {
                            seen_signatures.insert(signature, idx);
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
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        format!("{:x}", hasher.finalize())
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
            println!("  {} {} URLs, {} folders", browser.name(), url_count, folder_count);
        }
        
        let merged_urls = Self::count_all_bookmarks(merged);
        let merged_folders = Self::count_all_folders(merged);
        println!("  ─────────────────────────────────────────");
        println!("  Merged: {} URLs, {} folders", merged_urls, merged_folders);
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
                                report.add_validation_failed(browser_type, "Invalid bookmark structure");
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
                    println!("     Path: {:?}", path);
                }
                Err(_) => {
                    println!("  ❌ {} (not detected)", browser_type.name());
                }
            }
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
        Ok(())
    }
    
    
    pub async fn sync_history(&mut self, days: Option<i32>, dry_run: bool, verbose: bool) -> Result<()> {
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
                debug!("{} does not support history sync", adapter.browser_type().name());
                continue;
            }
            
            let browser_type = adapter.browser_type();
            match adapter.read_history(days) {
                Ok(history) => {
                    info!("✅ Read {} history items from {}", history.len(), browser_type.name());
                    browser_history.insert(browser_type, history);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read history from {}: {}", browser_type.name(), e);
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
                Ok(_) => {
                    info!("✅ Wrote history to {}", browser_type.name());
                }
                Err(e) => {
                    error!("❌ Failed to write history to {}: {}", browser_type.name(), e);
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
                debug!("{} does not support reading list sync", adapter.browser_type().name());
                continue;
            }
            
            let browser_type = adapter.browser_type();
            match adapter.read_reading_list() {
                Ok(items) => {
                    info!("✅ Read {} reading list items from {}", items.len(), browser_type.name());
                    browser_reading_lists.insert(browser_type, items);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read reading list from {}: {}", browser_type.name(), e);
                }
            }
        }
        
        if browser_reading_lists.is_empty() {
            warn!("⚠️  No reading lists could be read from any browser");
            return Ok(());
        }
        
        info!("🔄 Phase 2: Merging reading lists");
        let merged = self.merge_reading_lists(&browser_reading_lists, verbose)?;
        info!("📊 Merged result: {} unique reading list items", merged.len());
        
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
                Ok(_) => {
                    info!("✅ Wrote reading list to {}", browser_type.name());
                }
                Err(e) => {
                    error!("❌ Failed to write reading list to {}: {}", browser_type.name(), e);
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
                debug!("Processing {} history items from {}", history.len(), browser.name());
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
        merged.sort_by(|a, b| {
            b.last_visit.unwrap_or(0).cmp(&a.last_visit.unwrap_or(0))
        });

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
                debug!("Processing {} reading list items from {}", items.len(), browser.name());
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
        merged.sort_by(|a, b| {
            b.date_added.unwrap_or(0).cmp(&a.date_added.unwrap_or(0))
        });

        Ok(merged)
    }
    
    pub async fn sync_cookies(&mut self, dry_run: bool, verbose: bool) -> Result<()> {
        info!("🍪 Starting cookies synchronization");
        
        info!("📖 Phase 1: Reading cookies from all browsers");
        let mut browser_cookies = HashMap::new();
        
        for adapter in &self.adapters {
            if !adapter.supports_cookies() {
                debug!("{} does not support cookies sync", adapter.browser_type().name());
                continue;
            }
            
            let browser_type = adapter.browser_type();
            match adapter.read_cookies() {
                Ok(cookies) => {
                    info!("✅ Read {} cookies from {}", cookies.len(), browser_type.name());
                    browser_cookies.insert(browser_type, cookies);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read cookies from {}: {}", browser_type.name(), e);
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
                Ok(_) => {
                    info!("✅ Wrote cookies to {}", browser_type.name());
                }
                Err(e) => {
                    error!("❌ Failed to write cookies to {}: {}", browser_type.name(), e);
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
                    debug!("{} does not support cookies sync", adapter.browser_type().name());
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
                        let sample: Vec<_> = cookies.iter().take(5).map(|c| c.host.as_str()).collect();
                        debug!("   Sample hosts: {}", sample.join(", "));
                    }
                    all_cookies.extend(cookies);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read cookies from {}: {}", browser_type.name(), e);
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
        
        info!("📊 Deduplication: {} total → {} unique ({} duplicates removed)", 
            initial_count, merged_count, duplicates_removed);
        
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
        let brave_nightly_adapter = self.adapters.iter()
            .find(|a| a.browser_type() == BrowserType::BraveNightly);
            
        if let Some(adapter) = brave_nightly_adapter {
            match adapter.write_cookies(&all_cookies) {
                Ok(_) => {
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
        let waterfox_adapter = self.adapters.iter()
            .find(|a| a.browser_type() == BrowserType::Waterfox);
            
        if let Some(adapter) = waterfox_adapter {
            match adapter.write_cookies(&hub_cookies) {
                Ok(_) => {
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
                debug!("Processing {} cookies from {}", cookies.len(), browser.name());
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
        
        let html_content = std::fs::read_to_string(html_path)
            .context("Failed to read HTML file")?;
        
        let bookmarks = parse_safari_html(&html_content)?;
        info!("✅ Parsed {} bookmarks from HTML", bookmarks.len());
        
        if target == "all" {
            info!("📝 Writing to all browsers...");
            for adapter in &self.adapters {
                let browser_type = adapter.browser_type();
                match adapter.write_bookmarks(&bookmarks) {
                    Ok(_) => info!("✅ Wrote to {}", browser_type.name()),
                    Err(e) => error!("❌ Failed to write to {}: {}", browser_type.name(), e),
                }
            }
        } else {
            info!("📝 Writing to {}...", target);
            // Find specific browser
            for adapter in &self.adapters {
                if adapter.browser_type().name().to_lowercase().contains(&target.to_lowercase()) {
                    adapter.write_bookmarks(&bookmarks)?;
                    info!("✅ Wrote to {}", adapter.browser_type().name());
                    break;
                }
            }
        }
        
        Ok(())
    }

    /// Set hub browsers - migrate all data to hubs and optionally clear non-hub browsers
    pub async fn set_hub_browsers(
        &mut self,
        hub_names: &str,
        sync_history: bool,
        sync_reading_list: bool,
        sync_cookies: bool,
        clear_others: bool,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
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
                info!("  {} : {} URLs, {} folders", adapter.browser_type().name(), url_count, folder_count);
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
                        info!("  {} : {} history items", adapter.browser_type().name(), history.len());
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
                        info!("  {} : {} reading list items", adapter.browser_type().name(), items.len());
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
                        info!("  {} : {} cookies", adapter.browser_type().name(), cookies.len());
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
        info!("  📚 Merged bookmarks: {} URLs, {} folders", merged_urls, merged_folders);
        
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
            println!("    📚 {} bookmarks ({} folders)", merged_urls, merged_folders);
            if sync_history { println!("    📜 {} history items", merged_history.len()); }
            if sync_reading_list { println!("    📖 {} reading list items", merged_reading_list.len()); }
            if sync_cookies { println!("    🍪 {} cookies", merged_cookies.len()); }
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
                info!("  ✅ Backup: {} -> {:?}", adapter.browser_type().name(), path);
            }
        }
        
        // Phase 4: Write to hub browsers
        info!("\n✍️  Phase 4: Writing to hub browsers...");
        for adapter in &hub_adapters {
            let browser_name = adapter.browser_type().name();
            
            // Write bookmarks
            match adapter.write_bookmarks(&merged_bookmarks) {
                Ok(_) => info!("  ✅ {} : bookmarks written", browser_name),
                Err(e) => error!("  ❌ {} : failed to write bookmarks: {}", browser_name, e),
            }
            
            // Write history
            if sync_history && adapter.supports_history() {
                match adapter.write_history(&merged_history) {
                    Ok(_) => info!("  ✅ {} : history written", browser_name),
                    Err(e) => warn!("  ⚠️  {} : failed to write history: {}", browser_name, e),
                }
            }
            
            // Write reading list
            if sync_reading_list && adapter.supports_reading_list() {
                match adapter.write_reading_list(&merged_reading_list) {
                    Ok(_) => info!("  ✅ {} : reading list written", browser_name),
                    Err(e) => warn!("  ⚠️  {} : failed to write reading list: {}", browser_name, e),
                }
            }
            
            // Write cookies
            if sync_cookies && adapter.supports_cookies() {
                match adapter.write_cookies(&merged_cookies) {
                    Ok(_) => info!("  ✅ {} : cookies written", browser_name),
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
                match adapter.write_bookmarks(&empty_bookmarks) {
                    Ok(_) => info!("  ✅ {} : bookmarks cleared", browser_name),
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
                info!("  ✅ {} : {} URLs, {} folders", adapter.browser_type().name(), url_count, folder_count);
            }
        }
        
        println!("\n📊 Hub Configuration Complete!");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Hub browsers: {:?}", hub_list);
        println!("  Bookmarks: {} URLs, {} folders", merged_urls, merged_folders);
        if sync_history { println!("  History: {} items synced", merged_history.len()); }
        if sync_reading_list { println!("  Reading list: {} items synced", merged_reading_list.len()); }
        if sync_cookies { println!("  Cookies: {} items synced", merged_cookies.len()); }
        if clear_others { println!("  Non-hub browsers: CLEARED"); }
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
            anyhow::bail!("No matching browsers found for: {:?}", browser_list);
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
                        info!("  ✅ {} : found folder with {} bookmarks", browser_type.name(), count);
                    } else {
                        info!("  ⚠️  {} : scenario folder not found", browser_type.name());
                    }
                    scenario_folders.insert(browser_type, folder);
                }
                Err(e) => {
                    warn!("  ❌ {} : failed to read bookmarks: {}", browser_type.name(), e);
                }
            }
        }
        
        // Merge scenario folders
        info!("\n🔄 Phase 2: Merging scenario folders...");
        let merged_folder = self.merge_scenario_folders(&scenario_folders, scenario_path, verbose)?;
        let merged_count = Self::count_all_bookmarks(&merged_folder.children);
        info!("  📊 Merged folder contains {} bookmarks", merged_count);
        
        if dry_run {
            info!("\n🏃 Dry run mode - no changes will be made");
            println!("\n📊 Scenario Sync Preview:");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  Scenario: {}", scenario_path);
            println!("  Merged bookmarks: {}", merged_count);
            println!("  Target browsers:");
            for adapter in &target_adapters {
                println!("    - {}", adapter.browser_type().name());
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            return Ok(());
        }
        
        // Backup and write
        info!("\n💾 Phase 3: Creating backups...");
        for adapter in &target_adapters {
            if let Ok(path) = adapter.backup_bookmarks() {
                info!("  ✅ Backup: {:?}", path);
            }
        }
        
        info!("\n✍️  Phase 4: Updating scenario folders...");
        for adapter in &target_adapters {
            match adapter.read_bookmarks() {
                Ok(mut bookmarks) => {
                    // Replace or create scenario folder
                    if Self::replace_folder_by_path(&mut bookmarks, scenario_path, &merged_folder) {
                        match adapter.write_bookmarks(&bookmarks) {
                            Ok(_) => info!("  ✅ {} : scenario folder updated", adapter.browser_type().name()),
                            Err(e) => error!("  ❌ {} : failed to write: {}", adapter.browser_type().name(), e),
                        }
                    } else {
                        warn!("  ⚠️  {} : failed to locate/create scenario folder", adapter.browser_type().name());
                    }
                }
                Err(e) => error!("  ❌ {} : failed to read bookmarks: {}", adapter.browser_type().name(), e),
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

    fn find_folder_recursive(bookmarks: &[Bookmark], parts: &[&str], depth: usize) -> Option<Bookmark> {
        if depth >= parts.len() {
            return None;
        }
        
        let target_name = parts[depth].trim().to_lowercase();
        
        for bookmark in bookmarks {
            if bookmark.folder && bookmark.title.to_lowercase() == target_name {
                if depth == parts.len() - 1 {
                    // Found the target folder
                    return Some(bookmark.clone());
                } else {
                    // Continue searching in children
                    return Self::find_folder_recursive(&bookmark.children, parts, depth + 1);
                }
            }
        }
        
        None
    }

    /// Replace a folder at the specified path, or create it if it doesn't exist
    fn replace_folder_by_path(bookmarks: &mut Vec<Bookmark>, path: &str, new_folder: &Bookmark) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        Self::replace_folder_recursive(bookmarks, &parts, 0, new_folder)
    }

    fn replace_folder_recursive(bookmarks: &mut Vec<Bookmark>, parts: &[&str], depth: usize, new_folder: &Bookmark) -> bool {
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
                } else {
                    // Continue searching in children
                    return Self::replace_folder_recursive(&mut bookmark.children, parts, depth + 1, new_folder);
                }
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
                    debug!("  {} : {} bookmarks in scenario folder", browser.name(), count);
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
            let browser_list: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect();
            
            self.adapters.iter()
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
            
            match adapter.read_bookmarks() {
                Ok(mut bookmarks) => {
                    let initial_count = Self::count_all_bookmarks(&bookmarks);
                    let initial_folders = Self::count_all_folders(&bookmarks);
                    
                    info!("\n📊 {} : {} bookmarks, {} folders", browser_name, initial_count, initial_folders);
                    
                    let mut stats = CleanupStats::default();
                    
                    // Step 1: Remove duplicates with smart selection
                    if remove_duplicates {
                        Self::deduplicate_bookmarks_global(&mut bookmarks);
                        let after_dedup = Self::count_all_bookmarks(&bookmarks);
                        stats.duplicates_removed = initial_count.saturating_sub(after_dedup);
                        
                        if stats.duplicates_removed > 0 {
                            info!("  🔄 Removed {} duplicate bookmarks", stats.duplicates_removed);
                        }
                    }
                    
                    // Step 2: Remove empty folders
                    if remove_empty_folders {
                        stats.empty_folders_removed = Self::remove_empty_folders(&mut bookmarks);
                        
                        if stats.empty_folders_removed > 0 {
                            info!("  🗑️  Removed {} empty folders", stats.empty_folders_removed);
                        }
                    }
                    
                    let final_count = Self::count_all_bookmarks(&bookmarks);
                    let final_folders = Self::count_all_folders(&bookmarks);
                    
                    if dry_run {
                        info!("  🏃 Dry run - would remove {} duplicates, {} empty folders", 
                              stats.duplicates_removed, stats.empty_folders_removed);
                    } else if stats.duplicates_removed > 0 || stats.empty_folders_removed > 0 {
                        // Backup first
                        if let Ok(backup_path) = adapter.backup_bookmarks() {
                            info!("  💾 Backup created: {:?}", backup_path);
                        }
                        
                        // Write cleaned bookmarks
                        match adapter.write_bookmarks(&bookmarks) {
                            Ok(_) => {
                                info!("  ✅ Cleanup complete: {} bookmarks, {} folders remaining", 
                                      final_count, final_folders);
                            }
                            Err(e) => {
                                error!("  ❌ Failed to write cleaned bookmarks: {}", e);
                            }
                        }
                    } else {
                        info!("  ✨ No cleanup needed - bookmarks are already clean!");
                    }
                }
                Err(e) => {
                    error!("  ❌ Failed to read bookmarks from {}: {}", browser_name, e);
                }
            }
        }
        
        info!("\n✅ Cleanup complete!");
        Ok(())
    }

    /// Organize homepage bookmarks into a dedicated folder
    /// Homepage = URL that is a root domain (e.g., https://example.com or https://example.com/)
    pub async fn organize_homepages(
        &mut self,
        browser_names: Option<&str>,
        dry_run: bool,
        _verbose: bool,
    ) -> Result<()> {
        info!("📋 Starting homepage organization");
        
        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect();
            
            self.adapters.iter()
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
            
            match adapter.read_bookmarks() {
                Ok(mut bookmarks) => {
                    info!("\n📊 {} : Processing...", browser_name);
                    
                    // Collect all homepages from entire tree first
                    let mut homepages_collected: Vec<Bookmark> = Vec::new();
                    Self::collect_homepages_recursive(&mut bookmarks, &mut homepages_collected);

                    let moved_count = homepages_collected.len();

                    if moved_count > 0 {
                        // Find or create "网站主页" folder at root level
                        let homepage_folder = bookmarks.iter_mut()
                            .find(|b| b.folder && b.title == "网站主页");

                        if let Some(folder) = homepage_folder {
                            folder.children.extend(homepages_collected);
                        } else {
                            let new_folder = Bookmark {
                                id: format!("homepage-folder-{}", chrono::Utc::now().timestamp_millis()),
                                title: "网站主页".to_string(),
                                url: None,
                                folder: true,
                                children: homepages_collected,
                                date_added: Some(chrono::Utc::now().timestamp_millis()),
                                date_modified: Some(chrono::Utc::now().timestamp_millis()),
                            };
                            bookmarks.push(new_folder);
                        }
                        info!("  📁 Moved {} homepage bookmarks to root \"Homepages\" folder", moved_count);
                    } else {
                        info!("  ✨ No homepages found to organize");
                    }

                    if dry_run {
                        info!("  🏃 Dry run - would move {} homepages to root folder", moved_count);
                    } else if moved_count > 0 {
                        // Backup first
                        if let Ok(backup_path) = adapter.backup_bookmarks() {
                            info!("  💾 Backup created: {:?}", backup_path);
                        }
                        
                        // Write organized bookmarks
                        match adapter.write_bookmarks(&bookmarks) {
                            Ok(_) => {
                                info!("  ✅ Organization complete");
                            }
                            Err(e) => {
                                error!("  ❌ Failed to write organized bookmarks: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("  ❌ Failed to read bookmarks from {}: {}", browser_name, e);
                }
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
            if bookmark.folder && bookmark.title != "网站主页" && !bookmark.children.is_empty() {
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
    /// Examples: https://example.com, https://example.com/, http://example.com
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
    #[allow(dead_code)]
    fn find_empty_folders(bookmarks: &[Bookmark], path: &str, results: &mut Vec<String>) {
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
}

impl ClassificationRule {
    fn new(
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
        Self {
            name: name.to_string(),
            folder_name: folder_name.to_string(),
            folder_name_en: folder_name_en.to_string(),
            // Pre-lowercase all patterns for performance optimization
            url_patterns: url_patterns.iter().map(|s| s.to_lowercase()).collect(),
            domain_patterns: domain_patterns.iter().map(|s| s.to_lowercase()).collect(),
            path_patterns: path_patterns.iter().map(|s| s.to_lowercase()).collect(),
            title_patterns: title_patterns.iter().map(|s| s.to_lowercase()).collect(),
            priority,
            description: description.to_string(),
        }
    }
    
    /// Check if a bookmark matches this rule (optimized)
    fn matches(&self, url: &str, title: &str) -> bool {
        let url_lower = url.to_lowercase();
        let title_lower = title.to_lowercase();
        
        // Extract domain and path from URL
        let (domain, path) = Self::parse_url_parts(&url_lower);
        
        // Check URL patterns (already lowercase)
        for pattern in &self.url_patterns {
            if url_lower.contains(pattern) {
                return true;
            }
        }
        
        // Check domain patterns (already lowercase)
        for pattern in &self.domain_patterns {
            if domain.contains(pattern) {
                return true;
            }
        }
        
        // Check path patterns (already lowercase)
        for pattern in &self.path_patterns {
            if path.contains(pattern) {
                return true;
            }
        }
        
        // Check title patterns (already lowercase)
        for pattern in &self.title_patterns {
            if title_lower.contains(pattern) {
                return true;
            }
        }
        
        false
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
}

/// Built-in classification rules
pub fn get_builtin_rules() -> Vec<ClassificationRule> {
    vec![
        // 1. Login/Authentication pages
        ClassificationRule::new(
            "login",
            "登录入口",
            "Login Portals",
            vec!["login", "signin", "sign-in", "sign_in", "auth", "sso", "oauth", "accounts."],
            vec!["login.", "auth.", "sso.", "id.", "account.", "accounts."],
            vec!["/login", "/signin", "/sign-in", "/auth", "/sso", "/oauth", "/account/login"],
            vec!["登录", "登入", "sign in", "log in"],
            100,
            "Login and authentication pages"
        ),
        
        // 2. Social Media & Messaging
        ClassificationRule::new(
            "social",
            "社交媒体",
            "Social Media",
            vec![],
            vec![
                "twitter.com", "x.com", "facebook.com", "instagram.com", "linkedin.com",
                "weibo.com", "weixin.qq.com", "douyin.com", "tiktok.com", "reddit.com",
                "discord.com", "telegram.org", "whatsapp.com", "snapchat.com",
                "pinterest.com", "tumblr.com", "mastodon.", "threads.net",
                // Telegram
                "t.me", "telegram.me", "telegra.ph",
                // Reddit short
                "redd.it",
                // Fediverse
                "misskey.io", "pleroma.", "lemmy.",
                // VK
                "vk.com"
            ],
            vec![],
            vec![],
            90,
            "Social media platforms"
        ),
        
        // 3. Video/Streaming
        ClassificationRule::new(
            "video",
            "视频流媒体",
            "Video & Streaming",
            vec![],
            vec![
                "youtube.com", "youtu.be", "bilibili.com", "netflix.com", "hulu.com",
                "disneyplus.com", "primevideo.com", "twitch.tv", "vimeo.com",
                "iqiyi.com", "youku.com", "v.qq.com", "mgtv.com", "tv.sohu.com"
            ],
            vec!["/video", "/watch", "/play"],
            vec![],
            85,
            "Video and streaming platforms"
        ),
        
        // 4. Development Tools
        ClassificationRule::new(
            "dev",
            "开发工具",
            "Development Tools",
            vec![],
            vec![
                "github.com", "gitlab.com", "bitbucket.org", "stackoverflow.com",
                "codepen.io", "jsfiddle.net", "codesandbox.io", "replit.com",
                "npmjs.com", "crates.io", "pypi.org", "rubygems.org",
                "hub.docker.com", "vercel.com", "netlify.com", "heroku.com",
                "aws.amazon.com", "console.cloud.google.com", "portal.azure.com",
                "developer.mozilla.org", "devdocs.io", "docs.rs",
                // 开源代码托管
                "codeberg.org", "sourceforge.net", "sr.ht", "gitea.com",
                "gitlab.gnome.org", "gitlab.freedesktop.org", "invent.kde.org",
                "git.sr.ht", "0xacab.org", "framagit.org",
                // 浏览器扩展/脚本
                "greasyfork.org", "openuserjs.org", "userscripts-mirror.org",
                "addons.mozilla.org", "chrome.google.com/webstore",
                "mybrowseraddon.com", "webextension.org",
                // Colab/Jupyter
                "colab.research.google.com", "jupyter.org", "kaggle.com",
                // 新增
                "gist.github.com", "readthedocs.io", "gitbook.io",
                "dev.to", "hashnode.dev", "hackernoon.com"
            ],
            vec!["/api/", "/docs/", "/documentation", "/developer", "/sdk"],
            vec!["api 文档", "api doc", "developer", "开发者"],
            80,
            "Development and programming tools"
        ),
        
        // 5. Shopping/E-commerce
        ClassificationRule::new(
            "shopping",
            "购物网站",
            "Shopping",
            vec!["cart", "checkout", "shop.", "store."],
            vec![
                "amazon.", "ebay.com", "aliexpress.com", "taobao.com", "tmall.com",
                "jd.com", "pinduoduo.com", "shopify.com", "etsy.com", "walmart.com",
                "target.com", "bestbuy.com", "newegg.com"
            ],
            vec!["/cart", "/checkout", "/shop", "/product", "/item"],
            vec!["购物", "商城", "店铺", "shop", "store"],
            75,
            "E-commerce and shopping sites"
        ),
        
        // 6. News/Media
        ClassificationRule::new(
            "news",
            "新闻资讯",
            "News & Media",
            vec![],
            vec![
                "news.google.com", "cnn.com", "bbc.com", "reuters.com", "nytimes.com",
                "theguardian.com", "wsj.com", "bloomberg.com", "cnbc.com",
                "sina.com.cn", "163.com", "sohu.com", "qq.com/news", "ifeng.com",
                "thepaper.cn", "36kr.com", "huxiu.com"
            ],
            vec!["/news", "/article", "/story"],
            vec!["新闻", "资讯", "news", "breaking"],
            70,
            "News and media sites"
        ),
        
        // 7. Documentation/Reference
        ClassificationRule::new(
            "docs",
            "文档参考",
            "Documentation",
            vec!["docs.", "documentation.", "wiki.", "manual."],
            vec![
                "wikipedia.org", "wikimedia.org", "readthedocs.io", "gitbook.io"
            ],
            vec!["/docs", "/wiki", "/manual", "/guide", "/tutorial", "/reference", "/help"],
            vec!["文档", "手册", "教程", "指南", "documentation", "manual", "guide"],
            65,
            "Documentation and reference materials"
        ),
        
        // 8. Cloud Storage
        ClassificationRule::new(
            "cloud",
            "云存储",
            "Cloud Storage",
            vec![],
            vec![
                "drive.google.com", "dropbox.com", "onedrive.live.com", "box.com",
                "mega.nz", "mega.io", "swisstransfer.com",
                "icloud.com", "pan.baidu.com", "weiyun.com", "115.com", "mega.nz"
            ],
            vec!["/drive", "/files", "/storage"],
            vec!["云盘", "网盘", "cloud drive"],
            60,
            "Cloud storage services"
        ),
        
        // 9. Email/Communication
        ClassificationRule::new(
            "email",
            "邮箱通讯",
            "Email & Communication",
            vec!["mail.", "webmail."],
            vec![
                "mail.google.com", "outlook.live.com", "mail.yahoo.com",
                "mail.163.com", "mail.qq.com", "mail.sina.com",
                "protonmail.com", "tutanota.com", "zoho.com/mail"
            ],
            vec!["/mail", "/inbox", "/email"],
            vec!["邮箱", "邮件", "email", "inbox"],
            55,
            "Email and communication services"
        ),
        
        // 10. Finance/Banking
        ClassificationRule::new(
            "finance",
            "金融理财",
            "Finance & Banking",
            vec!["bank.", "banking.", "invest.", "trade."],
            vec![
                "paypal.com", "stripe.com", "wise.com", "venmo.com",
                "chase.com", "wellsfargo.com", "bankofamerica.com",
                "icbc.com.cn", "ccb.com", "boc.cn", "abchina.com",
                "alipay.com", "pay.weixin.qq.com"
            ],
            vec!["/banking", "/account", "/finance", "/invest", "/trade"],
            vec!["银行", "理财", "投资", "支付", "banking", "payment"],
            50,
            "Finance and banking services"
        ),
        
        // 11. AI/Tools
        ClassificationRule::new(
            "ai",
            "AI工具",
            "AI Tools",
            vec!["ai.", "gpt", "llm", "chat."],
            vec![
                "chat.openai.com", "openai.com", "anthropic.com", "claude.ai",
                "bard.google.com", "gemini.google.com", "copilot.microsoft.com",
                "midjourney.com", "stability.ai", "huggingface.co",
                "perplexity.ai", "poe.com", "character.ai"
            ],
            vec!["/chat", "/ai", "/generate"],
            vec!["chatgpt", "ai助手", "人工智能", "机器学习"],
            45,
            "AI and machine learning tools"
        ),
        
        // 12. Design/Creative
        ClassificationRule::new(
            "design",
            "设计创意",
            "Design & Creative",
            vec![],
            vec![
                "figma.com", "sketch.com", "canva.com", "adobe.com",
                "dribbble.com", "behance.net", "unsplash.com", "pexels.com",
                "pixabay.com", "freepik.com", "icons8.com"
            ],
            vec!["/design", "/creative", "/art", "/photo"],
            vec!["设计", "创意", "素材", "图片", "design", "creative"],
            40,
            "Design and creative tools"
        ),
        
        // 13. Education/Learning
        ClassificationRule::new(
            "education",
            "教育学习",
            "Education & Learning",
            vec!["learn.", "course.", "edu.", "study."],
            vec![
                "coursera.org", "udemy.com", "edx.org", "khanacademy.org",
                "duolingo.com", "codecademy.com", "udacity.com",
                "mooc.cn", "xuetangx.com", "icourse163.org"
            ],
            vec!["/course", "/learn", "/tutorial", "/lesson"],
            vec!["课程", "学习", "教程", "培训", "course", "learn", "tutorial"],
            35,
            "Education and learning platforms"
        ),
        
        // 14. Music/Audio
        ClassificationRule::new(
            "music",
            "音乐音频",
            "Music & Audio",
            vec![],
            vec![
                "spotify.com", "music.apple.com", "soundcloud.com",
                "music.163.com", "y.qq.com", "kugou.com", "kuwo.cn",
                "podcasts.apple.com", "podcasts.google.com"
            ],
            vec!["/music", "/audio", "/podcast", "/playlist"],
            vec!["音乐", "播客", "music", "podcast", "playlist"],
            30,
            "Music and audio platforms"
        ),
        
        // 15. Gaming
        ClassificationRule::new(
            "gaming",
            "游戏娱乐",
            "Gaming",
            vec!["game.", "games."],
            vec![
                "store.steampowered.com", "epicgames.com", "gog.com",
                "playstation.com", "xbox.com", "nintendo.com",
                "itch.io", "roblox.com", "minecraft.net"
            ],
            vec!["/game", "/games", "/play"],
            vec!["游戏", "game", "gaming", "play"],
            25,
            "Gaming platforms and game-related sites"
        ),
        
        // 16. Forums/Communities
        ClassificationRule::new(
            "forum",
            "论坛社区",
            "Forums & Communities",
            vec!["forum.", "bbs.", "community.", "forums."],
            vec![
                "reddit.com", "quora.com", "zhihu.com", "tieba.baidu.com",
                "v2ex.com", "segmentfault.com", "juejin.cn",
                "forums.mydigitallife.net", "bbs.pcbeta.com"
            ],
            vec!["/forum", "/community", "/discuss", "/topic"],
            vec!["论坛", "社区", "讨论", "forum", "community", "discuss"],
            20,
            "Forums and online communities"
        ),
        
        // 17. Dashboard/Admin
        ClassificationRule::new(
            "admin",
            "管理后台",
            "Admin & Dashboard",
            vec!["admin.", "dashboard.", "console.", "manage.", "panel."],
            vec![],
            vec!["/admin", "/dashboard", "/console", "/manage", "/backend", "/cms"],
            vec!["管理", "后台", "控制台", "admin", "dashboard", "manage"],
            15,
            "Admin panels and dashboards"
        ),
        
        // 18. API/Services
        ClassificationRule::new(
            "api",
            "API服务",
            "API & Services",
            vec!["api.", "gateway.", "service."],
            vec![],
            vec!["/api/", "/v1/", "/v2/", "/graphql", "/rest"],
            vec!["api", "接口", "服务"],
            10,
            "API endpoints and web services"
        ),
        
        // 19. App Stores
        ClassificationRule::new(
            "appstore",
            "应用商店",
            "App Stores",
            vec![],
            vec![
                "apps.apple.com", "play.google.com", "apps.microsoft.com",
                "f-droid.org", "apkpure.com", "apkmirror.com",
                "apps.kde.org", "flathub.org", "snapcraft.io",
                "modrinth.com", "curseforge.com", "itch.io",
                // 新增
                "alternativeto.net", "softpedia.com", "majorgeeks.com",
                "filehippo.com", "softonic.com", "cnet.com/download"
            ],
            vec!["/app/", "/apps/", "/store/", "/download/"],
            vec!["app store", "应用商店", "下载", "software"],
            55,
            "App stores and software distribution"
        ),
        
        // 20. Archives & References
        ClassificationRule::new(
            "archive",
            "存档资料",
            "Archives & References",
            vec![],
            vec![
                "archive.org", "web.archive.org", "archive.is", "archive.ph",
                "rentry.co", "rentry.org", "pastebin.com", "paste.ee",
                "ghostbin.com", "hastebin.com", "dpaste.org",
                "start.me", "linktr.ee",
                // 新增
                "notion.site", "coda.io", "airtable.com",
                "docs.google.com", "sheets.google.com"
            ],
            vec!["/archive", "/paste", "/doc/", "/document/"],
            vec!["archive", "存档", "备份", "文档"],
            45,
            "Web archives and paste services"
        ),
        
        // 21. Wiki & Knowledge Base
        ClassificationRule::new(
            "wiki",
            "百科知识",
            "Wiki & Knowledge",
            vec!["wiki."],
            vec![
                "wikipedia.org", "wikimedia.org", "fandom.com", "wikia.com",
                "wiki.archlinux.org", "wotaku.wiki", "wiki.gentoo.org",
                "bulbapedia.bulbagarden.net", "minecraft.wiki"
            ],
            vec!["/wiki/"],
            vec!["wiki", "百科", "encyclopedia"],
            50,
            "Wikis and knowledge bases"
        ),
        
        // 22. File Hosting & Cloud
        ClassificationRule::new(
            "filehost",
            "文件托管",
            "File Hosting",
            vec![],
            vec![
                "mega.nz", "mediafire.com", "zippyshare.com", "gofile.io",
                "anonfiles.com", "1fichier.com", "uploaded.net",
                "drive.google.com", "onedrive.live.com", "dropbox.com",
                "i.ibb.co", "imgur.com", "imgbb.com", "postimg.cc"
            ],
            vec!["/file/", "/download/", "/d/"],
            vec!["download", "下载", "文件"],
            40,
            "File hosting and cloud storage"
        ),
        
        // 23. Search Engines
        ClassificationRule::new(
            "search",
            "搜索引擎",
            "Search Engines",
            vec!["search."],
            vec![
                "google.com", "bing.com", "duckduckgo.com", "baidu.com",
                "yandex.com", "ecosia.org", "startpage.com", "searx.",
                "cse.google.com"
            ],
            vec!["/search"],
            vec!["search", "搜索"],
            35,
            "Search engines"
        ),
        
        // 24. NSFW/Adult Content (高优先级，确保被正确分类)
        ClassificationRule::new(
            "nsfw",
            "NSFW内容",
            "NSFW Content",
            vec!["porn", "xxx", "adult", "nsfw", "hentai", "sex", "nude", "erotic", "18+"],
            vec![
                "pornhub.com", "xvideos.com", "xnxx.com", "xhamster.com",
                "redtube.com", "youporn.com", "tube8.com", "spankbang.com",
                "eporner.com", "tnaflix.com", "drtuber.com", "sunporno.com",
                "porn.com", "4tube.com", "porntrex.com", "hqporner.com",
                "javlibrary.com", "javdb.com", "missav.com", "supjav.com",
                "hanime.tv", "nhentai.net", "e-hentai.org", "exhentai.org",
                "rule34.xxx", "gelbooru.com", "danbooru.donmai.us",
                "pixiv.net", "iwara.tv", "kemono.party", "coomer.party",
                "onlyfans.com", "fansly.com", "patreon.com/nsfw",
                "f95zone.to", "ulmf.org", "dlsite.com",
                "e621.net", "kemono.cr", "kemono.su", "baraag.net", "tbib.org"
            ],
            vec!["/porn", "/adult", "/xxx", "/nsfw", "/hentai", "/video/porn"],
            vec!["porn", "hentai", "nsfw", "adult", "xxx", "18+", "r18", "r-18"],
            95,  // 高优先级，仅次于登录页面
            "Adult and NSFW content"
        ),
        
        // 25. Discord & Chat Invites
        ClassificationRule::new(
            "discord",
            "Discord社群",
            "Discord Communities",
            vec![],
            vec![
                "discord.gg", "discord.com/invite", "discordapp.com/invite",
                "discord.me", "disboard.org", "top.gg", "discordapp.com"
            ],
            vec!["/invite/"],
            vec!["discord", "server", "invite"],
            88,
            "Discord server invites and communities"
        ),
        
        // 26. Anime & Manga
        ClassificationRule::new(
            "anime",
            "动漫二次元",
            "Anime & Manga",
            vec!["anime", "manga"],
            vec![
                "myanimelist.net", "anilist.co", "anidb.net", "kitsu.io",
                "mangadex.org", "mangaupdates.com", "mangakakalot.com",
                "crunchyroll.com", "funimation.com", "9anime.to",
                "gogoanime.io", "animixplay.to", "zoro.to",
                "theindex.moe", "everythingmoe.com", "everythingmoe.org",
                "wotaku.wiki", "asmr.one", "aidn.jp", "simkl.com",
                "newgrounds.com", "deviantart.com", "artstation.com"
            ],
            vec!["/anime/", "/manga/"],
            vec!["anime", "manga", "动漫", "漫画", "番剧"],
            72,
            "Anime and manga resources"
        ),
        
        // 27. Torrents & Downloads
        ClassificationRule::new(
            "torrents",
            "下载资源",
            "Downloads & Torrents",
            vec!["torrent", "magnet"],
            vec![
                "1337x.to", "nyaa.si", "rarbg.to", "thepiratebay.org",
                "rutracker.org", "torrentgalaxy.to", "limetorrents.info",
                "fitgirl-repacks.site", "dodi-repacks.site",
                "steamunlocked.net", "igg-games.com", "cs.rin.ru",
                "androidfilehost.com", "apkmirror.com"
            ],
            vec!["/torrent", "/download", "/magnet"],
            vec!["torrent", "download", "magnet", "repack"],
            28,
            "Torrent and download sites"
        ),
        
        // 28. Security & Privacy Tools
        ClassificationRule::new(
            "security",
            "安全隐私",
            "Security & Privacy",
            vec!["vpn", "proxy", "privacy"],
            vec![
                "mullvad.net", "protonvpn.com", "nordvpn.com", "expressvpn.com",
                "adguard.com", "adguard-dns.io", "rethinkdns.com",
                "virustotal.com", "malwarebytes.com", "eff.org",
                "privacytools.io", "privacyguides.org", "proton.me",
                "grc.com", "haveibeenpwned.com", "objective-see.org"
            ],
            vec!["/security", "/privacy", "/vpn"],
            vec!["vpn", "proxy", "privacy", "security", "安全", "隐私"],
            42,
            "Security and privacy tools"
        ),
        
        // 29. Linux & Open Source
        ClassificationRule::new(
            "linux",
            "Linux开源",
            "Linux & Open Source",
            vec![],
            vec![
                "archlinux.org", "wiki.archlinux.org", "aur.archlinux.org",
                "ubuntu.com", "debian.org", "fedoraproject.org",
                "linuxmint.com", "manjaro.org", "opensuse.org",
                "gnome.org", "kde.org", "apps.kde.org",
                "flathub.org", "snapcraft.io", "appimage.org",
                "gnu.org", "fsf.org", "opensource.org",
                "gitlab.gnome.org", "wiki.gnome.org", "apps.gnome.org",
                "invent.kde.org", "cdn.kde.org", "krita-artists.org", "0xacab.org"
            ],
            vec!["/linux", "/gnu"],
            vec!["linux", "gnu", "开源", "open source"],
            38,
            "Linux distributions and open source"
        ),
        
        // 30. Microsoft Services
        ClassificationRule::new(
            "microsoft",
            "微软服务",
            "Microsoft Services",
            vec![],
            vec![
                "microsoft.com", "support.microsoft.com", "answers.microsoft.com",
                "docs.microsoft.com", "learn.microsoft.com",
                "office.com", "office365.com", "live.com",
                "azure.microsoft.com", "visualstudio.com",
                "windows.com", "xbox.com"
            ],
            vec![],
            vec!["microsoft", "windows", "office", "azure"],
            36,
            "Microsoft products and services"
        ),
        
        // 31. Apple Services
        ClassificationRule::new(
            "apple",
            "苹果服务",
            "Apple Services",
            vec![],
            vec![
                "apple.com", "support.apple.com", "developer.apple.com",
                "icloud.com", "testflight.apple.com",
                "ios.cfw.guide", "ipsw.me", "appledb.dev"
            ],
            vec![],
            vec!["apple", "iphone", "ipad", "mac", "ios"],
            34,
            "Apple products and services"
        ),
        
        // 32. Google Services
        ClassificationRule::new(
            "google",
            "谷歌服务",
            "Google Services",
            vec![],
            vec![
                "google.com", "sites.google.com", "labs.google",
                "cloud.google.com", "firebase.google.com",
                "analytics.google.com", "ads.google.com",
                "workspace.google.com", "meet.google.com"
            ],
            vec![],
            vec!["google", "谷歌"],
            32,
            "Google products and services"
        ),
        
        // 33. Fediverse & Decentralized
        ClassificationRule::new(
            "fediverse",
            "联邦宇宙",
            "Fediverse & Decentralized",
            vec!["mastodon", "fediverse"],
            vec![
                "mastodon.social", "mastodon.online", "mstdn.social",
                "misskey.io", "pleroma.social", "lemmy.ml",
                "pixelfed.social", "peertube.social",
                "the-federation.info", "fedidb.org", "fediverse.party"
            ],
            vec![],
            vec!["fediverse", "mastodon", "activitypub"],
            30,
            "Fediverse and decentralized social networks"
        ),
        
        // 34. XDA & Mobile Dev
        ClassificationRule::new(
            "mobile",
            "移动开发",
            "Mobile Development",
            vec![],
            vec![
                "xdaforums.com", "xda-developers.com",
                "forum.mobilism.org", "forums.mydigitallife.net",
                "gbatemp.net", "pdalife.com",
                "apt.izzysoft.de"
            ],
            vec!["/forum", "/thread"],
            vec!["android", "rom", "root", "mod"],
            26,
            "Mobile development and modding"
        ),
        
        // 35. Science & Research
        ClassificationRule::new(
            "science",
            "科学研究",
            "Science & Research",
            vec![],
            vec![
                "nasa.gov", "arxiv.org", "nature.com", "science.org",
                "nih.gov", "si.edu", "libretexts.org",
                "wolframalpha.com", "mathworld.wolfram.com",
                "loc.gov", "ncatlab.org"
            ],
            vec!["/research", "/paper", "/article"],
            vec!["research", "science", "paper", "study"],
            24,
            "Science and research resources"
        ),
        
        // 36. Streaming & Live
        ClassificationRule::new(
            "streaming",
            "直播平台",
            "Streaming & Live",
            vec!["stream", "live"],
            vec![
                "twitch.tv", "kick.com", "youtube.com/live",
                "rivestream.org", "pomf.tv", "alienflix.net",
                "pluto.tv", "tubi.tv"
            ],
            vec!["/live", "/stream"],
            vec!["live", "stream", "直播"],
            68,
            "Live streaming platforms"
        ),
        
        // 37. Browser Extensions
        ClassificationRule::new(
            "extensions",
            "浏览器扩展",
            "Browser Extensions",
            vec![],
            vec![
                "add0n.com", "webextension.org", "mybrowseraddon.com",
                "userstyles.world", "betterdiscord.app",
                "draculatheme.com", "sindresorhus.com", "openuserjs.org"
            ],
            vec!["/extension", "/addon", "/theme"],
            vec!["extension", "addon", "theme", "扩展", "插件"],
            22,
            "Browser extensions and themes"
        ),
        
        // 38. Online Tools & Utilities
        ClassificationRule::new(
            "tools",
            "在线工具",
            "Online Tools",
            vec!["tool", "converter", "generator"],
            vec![
                "url-decode.com", "caniuse.com", "regex101.com",
                "jsonformatter.org", "base64decode.org",
                "time.is", "weather.com", "viewdns.info",
                "ss64.com", "softwareok.com", "nirsoft.net",
                "majorgeeks.com", "wolframalpha.com", "toptal.com",
                "neal.fun", "codepen.io",
                // 新增
                "builtwith.com", "fontmeme.com", "theuselessweb.com",
                "pointlesssites.com", "perchance.org", "pudding.cool"
            ],
            vec!["/tool", "/convert", "/generate", "/tools"],
            vec!["tool", "converter", "generator", "工具"],
            18,
            "Online tools and utilities"
        ),
        
        // 39. Productivity & Notes
        ClassificationRule::new(
            "productivity",
            "效率工具",
            "Productivity",
            vec![],
            vec![
                "notion.so", "notion.site", "obsidian.md",
                "trello.com", "airtable.com", "asana.com",
                "todoist.com", "evernote.com"
            ],
            vec![],
            vec!["note", "todo", "task", "笔记", "待办"],
            16,
            "Productivity and note-taking tools"
        ),
        
        // 40. Gaming Communities
        ClassificationRule::new(
            "gamecommunity",
            "游戏社区",
            "Gaming Communities",
            vec![],
            vec![
                "steamcommunity.com", "steamdb.info", "steambase.io",
                "crackwatch.com", "pcgamingwiki.com",
                "nexusmods.com", "moddb.com", "gamebanana.com",
                "lichess.org", "chess.com", "emulation.gametechwiki.com",
                "cs.rin.ru", "store.steampowered.com", "gamejolt.com", "modlist.in", "geode-sdk.org"
            ],
            vec!["/community", "/mod", "/guide"],
            vec!["mod", "guide", "wiki", "攻略"],
            14,
            "Gaming communities and resources"
        ),
        
        // 41. Image Hosting
        ClassificationRule::new(
            "imagehost",
            "图床托管",
            "Image Hosting",
            vec![],
            vec![
                "i.ibb.co", "ibb.co", "imgbb.com", "imgur.com",
                "postimg.cc", "imgbox.com", "flickr.com",
                "500px.com", "unsplash.com", "pexels.com"
            ],
            vec![],
            vec![],
            52,
            "Image hosting services"
        ),
        
        // 42. Link Aggregators & Directories
        ClassificationRule::new(
            "directories",
            "导航目录",
            "Directories & Aggregators",
            vec![],
            vec![
                "linktr.ee", "linktree.com", "start.me", "curlie.org",
                "fmhy.net", "rgshows.me", "alternativeto.net"
            ],
            vec![],
            vec!["directory", "list", "collection"],
            48,
            "Link directories and aggregators"
        ),
        
        // 43. Chinese Platforms
        ClassificationRule::new(
            "chinese",
            "中文平台",
            "Chinese Platforms",
            vec![],
            vec![
                "baidu.com", "zhihu.com", "zhuanlan.zhihu.com",
                "bilibili.com", "weibo.com", "douban.com",
                "linux.do", "v2ex.com", "juejin.cn"
            ],
            vec![],
            vec![],
            46,
            "Chinese language platforms"
        ),
        
        // 44. Design & Creative
        ClassificationRule::new(
            "creative",
            "设计素材",
            "Design & Creative",
            vec![],
            vec![
                "adobe.com", "icons8.com", "flaticon.com",
                "fontawesome.com", "fonts.google.com",
                "krita-artists.org", "blender.org"
            ],
            vec!["/design", "/icon", "/font"],
            vec!["design", "icon", "font", "素材"],
            44,
            "Design resources and creative tools"
        ),
        
        // 45. Hardware & Tech
        ClassificationRule::new(
            "hardware",
            "硬件技术",
            "Hardware & Tech",
            vec![],
            vec![
                "nvidia.com", "amd.com", "intel.com",
                "techpowerup.com", "tomshardware.com",
                "anandtech.com", "notebookcheck.net"
            ],
            vec![],
            vec!["gpu", "cpu", "hardware"],
            40,
            "Hardware and technology resources"
        ),
        
        // 46. Hosting Platforms (个人项目/博客)
        ClassificationRule::new(
            "hosting",
            "托管项目",
            "Hosted Projects",
            vec![],
            vec![
                "github.io", "vercel.app", "netlify.app", "pages.dev",
                "neocities.org", "glitch.me", "web.app", "appspot.com",
                "gitlab.io", "surge.sh", "fly.dev", "railway.app",
                "render.com", "heroku.com", "replit.com"
            ],
            vec![],
            vec![],
            8,  // 低优先级，作为兜底
            "Hosted projects and personal sites"
        ),
        
        // 47. Blogs & Personal Sites
        ClassificationRule::new(
            "blogs",
            "博客站点",
            "Blogs & Personal",
            vec!["blog"],
            vec![
                "blogspot.com", "wordpress.com", "medium.com",
                "substack.com", "ghost.io", "wixsite.com",
                "carrd.co", "tumblr.com"
            ],
            vec!["/blog", "/post"],
            vec!["blog", "博客"],
            10,
            "Blogs and personal websites"
        ),
        
        // 48. Developer Tools
        ClassificationRule::new(
            "devtools",
            "开发者工具",
            "Developer Tools",
            vec![],
            vec![
                "jetbrains.com", "cursor.com", "vscode.dev",
                "replit.com", "codepen.io", "jsfiddle.net",
                "codesandbox.io", "stackblitz.com"
            ],
            vec![],
            vec!["ide", "editor", "编辑器"],
            56,
            "Developer tools and IDEs"
        ),
        
        // === 新增规则 (49-75) ===
        
        // 49. DevOps & CI/CD
        ClassificationRule::new(
            "devops",
            "DevOps运维",
            "DevOps & CI/CD",
            vec!["jenkins", "gitlab-ci", "circleci", "travis", "actions"],
            vec![
                "jenkins.io", "circleci.com", "travis-ci.org", "travis-ci.com",
                "actions.github.com", "gitlab.com/ci", "drone.io",
                "teamcity.jetbrains.com", "bamboo.atlassian.com",
                "semaphoreci.com", "buildkite.com", "concourse-ci.org"
            ],
            vec!["/pipeline", "/ci", "/cd", "/deploy", "/builds"],
            vec!["CI/CD", "DevOps", "持续集成", "自动化部署", "pipeline"],
            76,
            "DevOps and CI/CD platforms"
        ),
        
        // 50. 数据库服务
        ClassificationRule::new(
            "database",
            "数据库服务",
            "Database Services",
            vec!["database", "db", "sql", "nosql"],
            vec![
                "postgresql.org", "mysql.com", "mongodb.com", "redis.io",
                "supabase.com", "firebase.google.com", "planetscale.com",
                "cockroachlabs.com", "cassandra.apache.org", "mariadb.org",
                "sqlite.org", "arangodb.com", "couchdb.apache.org",
                "neo4j.com", "influxdata.com", "timescale.com"
            ],
            vec!["/database", "/db", "/sql", "/query"],
            vec!["database", "数据库", "SQL", "NoSQL", "查询"],
            74,
            "Database services and tools"
        ),
        
        // 51. 区块链加密货币
        ClassificationRule::new(
            "blockchain",
            "区块链加密",
            "Blockchain & Crypto",
            vec!["crypto", "blockchain", "nft", "defi", "web3", "bitcoin", "ethereum"],
            vec![
                "ethereum.org", "bitcoin.org", "binance.com", "coinbase.com",
                "opensea.io", "uniswap.org", "metamask.io", "rarible.com",
                "crypto.com", "kraken.com", "gemini.com", "coinmarketcap.com",
                "coingecko.com", "etherscan.io", "blockchain.com"
            ],
            vec!["/crypto", "/blockchain", "/nft", "/defi", "/wallet"],
            vec!["加密货币", "区块链", "NFT", "DeFi", "Web3", "比特币", "以太坊"],
            54,
            "Blockchain and cryptocurrency"
        ),
        
        // 52. 服务器监控
        ClassificationRule::new(
            "monitoring",
            "服务器监控",
            "Server Monitoring",
            vec!["monitor", "metrics", "observability", "apm"],
            vec![
                "grafana.com", "prometheus.io", "datadog.com", "newrelic.com",
                "sentry.io", "uptimerobot.com", "pingdom.com", "statuspage.io",
                "elastic.co", "splunk.com", "dynatrace.com", "appdynamics.com"
            ],
            vec!["/monitor", "/metrics", "/dashboard", "/analytics"],
            vec!["监控", "性能", "metrics", "observability"],
            58,
            "Server monitoring and observability"
        ),
        
        // 53. API文档与测试
        ClassificationRule::new(
            "apitools",
            "API工具",
            "API Tools",
            vec!["postman", "insomnia", "swagger", "openapi"],
            vec![
                "postman.com", "insomnia.rest", "hoppscotch.io", "swagger.io",
                "stoplight.io", "apidoc.tools", "readme.com", "apidog.com"
            ],
            vec!["/api/docs", "/swagger", "/openapi", "/api-docs"],
            vec!["API测试", "Postman", "Swagger", "API文档"],
            62,
            "API documentation and testing tools"
        ),
        
        // 54. 容器与云原生
        ClassificationRule::new(
            "containers",
            "容器云原生",
            "Containers & Cloud Native",
            vec!["docker", "kubernetes", "k8s", "container"],
            vec![
                "docker.com", "kubernetes.io", "k8s.io", "helm.sh",
                "rancher.com", "portainer.io", "containerd.io",
                "podman.io", "cloud.docker.com", "docker.io"
            ],
            vec!["/docker", "/kubernetes", "/k8s", "/container"],
            vec!["容器", "Docker", "Kubernetes", "K8s", "云原生"],
            66,
            "Container and cloud-native technologies"
        ),
        
        // 55. 软件许可与开源
        ClassificationRule::new(
            "licensing",
            "开源许可",
            "Open Source Licensing",
            vec!["license", "licensing", "opensource"],
            vec![
                "choosealicense.com", "opensource.org", "creativecommons.org",
                "tldrlegal.com", "spdx.org", "gnu.org/licenses"
            ],
            vec!["/license", "/licensing"],
            vec!["开源许可", "License", "GPL", "MIT", "Apache"],
            33,
            "Open source licenses and legal"
        ),
        
        // 56. 旅游出行
        ClassificationRule::new(
            "travel",
            "旅游出行",
            "Travel & Tourism",
            vec!["travel", "trip", "hotel", "flight", "vacation"],
            vec![
                "booking.com", "airbnb.com", "expedia.com", "tripadvisor.com",
                "skyscanner.com", "hotels.com", "priceline.com", "kayak.com",
                "agoda.com", "hostelworld.com", "ctrip.com", "qunar.com",
                "mafengwo.cn", "qyer.com"
            ],
            vec!["/travel", "/trip", "/hotel", "/flight", "/vacation"],
            vec!["旅游", "酒店", "机票", "travel", "hotel", "flight"],
            41,
            "Travel and tourism platforms"
        ),
        
        // 57. 外卖美食
        ClassificationRule::new(
            "food",
            "外卖美食",
            "Food Delivery",
            vec!["food", "delivery", "restaurant", "menu"],
            vec![
                "doordash.com", "ubereats.com", "grubhub.com", "deliveroo.com",
                "ele.me", "meituan.com", "dianping.com", "zomato.com",
                "yelp.com", "opentable.com", "seamless.com"
            ],
            vec!["/food", "/delivery", "/restaurant", "/menu", "/order"],
            vec!["外卖", "美食", "restaurant", "food delivery", "订餐"],
            39,
            "Food delivery and restaurant services"
        ),
        
        // 58. 地图导航
        ClassificationRule::new(
            "maps",
            "地图导航",
            "Maps & Navigation",
            vec!["maps", "navigation", "directions"],
            vec![
                "maps.google.com", "maps.apple.com", "openstreetmap.org",
                "waze.com", "here.com", "mapbox.com", "map.baidu.com",
                "amap.com", "mapy.cz", "yandex.ru/maps"
            ],
            vec!["/maps", "/navigation", "/directions", "/route"],
            vec!["地图", "导航", "navigation", "directions", "maps"],
            53,
            "Maps and navigation services"
        ),
        
        // 59. 健康医疗
        ClassificationRule::new(
            "health",
            "健康医疗",
            "Health & Medical",
            vec!["health", "medical", "medicine", "doctor"],
            vec![
                "webmd.com", "healthline.com", "mayoclinic.org", "nih.gov",
                "medlineplus.gov", "drugs.com", "rxlist.com", "patient.info",
                "medicalnewstoday.com", "everydayhealth.com"
            ],
            vec!["/health", "/medical", "/medicine", "/doctor", "/symptom"],
            vec!["健康", "医疗", "health", "medical", "医生", "疾病"],
            47,
            "Health and medical information"
        ),
        
        // 60. 天气服务
        ClassificationRule::new(
            "weather",
            "天气服务",
            "Weather Services",
            vec!["weather", "forecast", "meteo"],
            vec![
                "weather.com", "accuweather.com", "weather.gov", "windy.com",
                "weatherunderground.com", "wunderground.com", "meteoblue.com",
                "weather.yahoo.com", "yr.no"
            ],
            vec!["/weather", "/forecast"],
            vec!["天气", "weather", "forecast", "气象"],
            31,
            "Weather forecast services"
        ),
        
        // 61. 求职招聘
        ClassificationRule::new(
            "jobs",
            "求职招聘",
            "Jobs & Careers",
            vec!["jobs", "career", "hiring", "recruit"],
            vec![
                "linkedin.com/jobs", "indeed.com", "glassdoor.com", "monster.com",
                "zhipin.com", "lagou.com", "51job.com", "liepin.com",
                "workable.com", "greenhouse.io", "lever.co"
            ],
            vec!["/jobs", "/career", "/careers", "/hiring", "/job"],
            vec!["招聘", "求职", "career", "hiring", "工作"],
            43,
            "Job search and career platforms"
        ),
        
        // 62. 播客Podcast
        ClassificationRule::new(
            "podcast",
            "播客节目",
            "Podcasts",
            vec!["podcast", "podcasts"],
            vec![
                "podcasts.apple.com", "podcasts.google.com", "spotify.com/podcasts",
                "anchor.fm", "podbean.com", "buzzsprout.com", "transistor.fm",
                "simplecast.com", "overcast.fm", "pocketcasts.com"
            ],
            vec!["/podcast", "/podcasts", "/episode", "/show"],
            vec!["podcast", "播客", "节目", "episode"],
            37,
            "Podcast platforms and directories"
        ),
        
        // 63. 电子书阅读
        ClassificationRule::new(
            "ebooks",
            "电子书阅读",
            "E-books & Reading",
            vec!["ebook", "books", "reading", "library"],
            vec![
                "kindle.amazon.com", "goodreads.com", "zlibrary.to", "z-lib.org",
                "annas-archive.org", "libgen.is", "libgen.rs", "libgen.st",
                "archive.org/details/books", "gutenberg.org", "openlibrary.org",
                "scribd.com", "bookwalker.jp"
            ],
            vec!["/book", "/books", "/ebook", "/read", "/library"],
            vec!["电子书", "ebook", "books", "阅读", "reading"],
            29,
            "E-book platforms and digital libraries"
        ),
        
        // 64. 漫画Comic
        ClassificationRule::new(
            "comics",
            "漫画在线",
            "Comics & Webcomics",
            vec!["comic", "webtoon", "webcomic", "manga"],
            vec![
                "webtoons.com", "comixology.com", "readcomiconline.li",
                "marvel.com", "dccomics.com", "tapas.io", "globalcomix.com",
                "comic-walker.com", "mangaplus.shueisha.co.jp"
            ],
            vec!["/comic", "/comics", "/webtoon", "/manga", "/chapter"],
            vec!["漫画", "comic", "webtoon", "webcomic"],
            27,
            "Comics and webcomics platforms"
        ),
        
        // 65. 摄影图片
        ClassificationRule::new(
            "photography",
            "摄影图片",
            "Photography",
            vec!["photo", "photography", "photographer"],
            vec![
                "500px.com", "flickr.com", "unsplash.com", "pexels.com",
                "pixabay.com", "shutterstock.com", "gettyimages.com",
                "smugmug.com", "photobucket.com", "imageshack.com"
            ],
            vec!["/photo", "/photos", "/image", "/photography", "/gallery"],
            vec!["摄影", "photography", "photo", "图片"],
            23,
            "Photography and image platforms"
        ),
        
        // 66. 体育运动
        ClassificationRule::new(
            "sports",
            "体育运动",
            "Sports",
            vec!["sport", "sports", "football", "basketball"],
            vec![
                "espn.com", "nba.com", "nfl.com", "fifa.com", "olympics.com",
                "mlb.com", "nhl.com", "uefa.com", "premierleague.com",
                "skysports.com", "bleacherreport.com", "sports.yahoo.com",
                "thescore.com", "livescore.com"
            ],
            vec!["/sports", "/sport", "/game", "/match", "/scores"],
            vec!["体育", "sports", "运动", "足球", "篮球"],
            21,
            "Sports and athletics"
        ),
        
        // 67. 二手交易
        ClassificationRule::new(
            "secondhand",
            "二手交易",
            "Secondhand & Marketplace",
            vec!["secondhand", "used", "marketplace", "resale"],
            vec![
                "ebay.com", "mercari.com", "poshmark.com", "depop.com",
                "xianyu.taobao.com", "zhuanzhuan.com", "craigslist.org",
                "offerup.com", "letgo.com", "facebook.com/marketplace"
            ],
            vec!["/marketplace", "/sell", "/buy", "/listing"],
            vec!["二手", "闲置", "secondhand", "marketplace", "转卖"],
            19,
            "Secondhand and marketplace platforms"
        ),
        
        // 68. 团购优惠
        ClassificationRule::new(
            "deals",
            "团购优惠",
            "Deals & Coupons",
            vec!["deal", "deals", "coupon", "discount", "promo"],
            vec![
                "groupon.com", "slickdeals.net", "dealnews.com", "smzdm.com",
                "retailmenot.com", "coupons.com", "honey.com", "rakuten.com",
                "fatwallet.com", "bensbargains.com", "dealsplus.com"
            ],
            vec!["/deal", "/deals", "/coupon", "/discount", "/promo"],
            vec!["优惠", "折扣", "deals", "coupon", "促销", "团购"],
            17,
            "Deals and coupon platforms"
        ),
        
        // 69. 价格比较
        ClassificationRule::new(
            "pricetracking",
            "价格比较",
            "Price Tracking",
            vec!["price", "compare", "tracking", "comparison"],
            vec![
                "camelcamelcamel.com", "keepa.com", "pricespy.com", "pricegrabber.com",
                "shopzilla.com", "nextag.com", "idealo.de", "geizhals.de"
            ],
            vec!["/price", "/compare", "/tracking", "/history"],
            vec!["价格", "比价", "price tracking", "comparison"],
            13,
            "Price comparison and tracking"
        ),
        
        // 70. 短链接服务
        ClassificationRule::new(
            "shorturl",
            "短链接服务",
            "URL Shorteners",
            vec!["shorten", "short", "tiny"],
            vec![
                "bit.ly", "tinyurl.com", "t.co", "goo.gl", "shorturl.at",
                "ow.ly", "is.gd", "buff.ly", "adf.ly", "bitly.com",
                "cutt.ly", "rebrandly.com", "soo.gd"
            ],
            vec![],
            vec!["短链接", "short url", "缩短"],
            12,
            "URL shortening services"
        ),
        
        // 71. 本地开发服务
        ClassificationRule::new(
            "localhost",
            "本地开发",
            "Local Development",
            vec!["localhost", "127.0.0.1", "0.0.0.0", "192.168.", ":3000", ":8080", ":5000", ":4200"],
            vec!["localhost", "127.0.0.1", "0.0.0.0"],
            vec![],
            vec!["本地", "local", "dev", "development"],
            11,
            "Local development servers"
        ),
        
        // 72. 翻译服务
        ClassificationRule::new(
            "translation",
            "翻译服务",
            "Translation Services",
            vec!["translate", "translation", "translator"],
            vec![
                "translate.google.com", "deepl.com", "bing.com/translator",
                "reverso.net", "linguee.com", "youdao.com", "fanyi.baidu.com",
                "translate.yandex.com", "papago.naver.com"
            ],
            vec!["/translate", "/translation", "/translator"],
            vec!["翻译", "translate", "translation", "翻译器"],
            49,
            "Translation and language services"
        ),
        
        // 73. 字体资源
        ClassificationRule::new(
            "fonts",
            "字体资源",
            "Fonts & Typography",
            vec!["font", "fonts", "typeface", "typography"],
            vec![
                "fonts.google.com", "fontsquirrel.com", "dafont.com", "fontspace.com",
                "1001fonts.com", "abstractfonts.com", "fontlibrary.org",
                "myfonts.com", "typography.com", "fonts.adobe.com"
            ],
            vec!["/font", "/fonts", "/typeface"],
            vec!["字体", "font", "typography", "typeface"],
            25,
            "Font and typography resources"
        ),
        
        // === 日语细分规则 (74-88) ===
        
        // 74. 日本购物
        ClassificationRule::new(
            "japanese_shopping",
            "日本购物",
            "Japanese Shopping",
            vec!["rakuten", "amazon.co.jp", ".co.jp"],
            vec![
                "rakuten.co.jp", "amazon.co.jp", "shopping.yahoo.co.jp",
                "mercari.com", "suruga-ya.jp", "bookoff.co.jp",
                "yodobashi.com", "biccamera.com", "kakaku.com"
            ],
            vec!["/shop", "/cart", "/product"],
            vec!["楽天", "購入", "ショッピング"],
            53,
            "Japanese e-commerce and shopping sites"
        ),
        
        // 75. 日本新闻
        ClassificationRule::new(
            "japanese_news",
            "日本新闻",
            "Japanese News",
            vec!["asahi", "yomiuri", "mainichi", "nikkei"],
            vec![
                "asahi.com", "yomiuri.co.jp", "mainichi.jp", "nikkei.com",
                "sankei.com", "jiji.com", "kyodo.co.jp", "nhk.or.jp",
                "47news.jp", "itmedia.co.jp"
            ],
            vec!["/news", "/article"],
            vec!["ニュース", "新聞", "記事"],
            52,
            "Japanese news and media sites"
        ),
        
        // 76. 日本娱乐视频
        ClassificationRule::new(
            "japanese_entertainment",
            "日本娱乐",
            "Japanese Entertainment",
            vec!["niconico", "abema", "tver"],
            vec![
                "nicovideo.jp", "abema.tv", "tver.jp", "gyao.yahoo.co.jp",
                "u-next.jp", "hulu.jp", "netflix.co.jp", "amazon.co.jp/prime",
                "dazn.com"
            ],
            vec!["/watch", "/video", "/anime"],
            vec!["ニコニコ", "動画", "アニメ", "番組"],
            51,
            "Japanese video streaming and entertainment"
        ),
        
        // 77. 日本社交平台
        ClassificationRule::new(
            "japanese_social",
            "日本社交",
            "Japanese Social",
            vec!["line", "mixi", "twitter.com/ja"],
            vec![
                "line.me", "mixi.jp", "ameba.jp", "fc2.com",
                "hatena.ne.jp", "note.com", "livedoor.blog"
            ],
            vec!["/profile", "/user", "/blog"],
            vec!["友達", "メッセージ", "ブログ"],
            50,
            "Japanese social media platforms"
        ),
        
        // 78. 日本科技开发
        ClassificationRule::new(
            "japanese_tech",
            "日本科技",
            "Japanese Tech",
            vec!["qiita", "zenn", "teratail"],
            vec![
                "qiita.com", "zenn.dev", "teratail.com", "atcoder.jp",
                "codepen.io", "github.com", "stackoverflow.com"
            ],
            vec!["/tech", "/dev", "/code"],
            vec!["技術", "プログラミング", "開発"],
            49,
            "Japanese tech and developer communities"
        ),
        
        // 79. 日本游戏
        ClassificationRule::new(
            "japanese_gaming",
            "日本游戏",
            "Japanese Gaming",
            vec!["dmm", "gree", "mobage"],
            vec![
                "dmm.co.jp", "dmm.com", "gree.jp", "mobage.jp",
                "4gamer.net", "famitsu.com", "dengeki.com",
                "nintendo.co.jp", "playstation.com/ja-jp"
            ],
            vec!["/game", "/play"],
            vec!["ゲーム", "プレイ", "攻略"],
            48,
            "Japanese gaming platforms and sites"
        ),
        
        // 80. 日本漫画小说
        ClassificationRule::new(
            "japanese_manga",
            "日本漫画",
            "Japanese Manga",
            vec!["pixiv", "booth", "fanbox"],
            vec![
                "pixiv.net", "booth.pm", "fanbox.cc", "seiga.nicovideo.jp",
                "comic.pixiv.net", "shonenjump.com", "comico.jp",
                "piccoma.com"
            ],
            vec!["/manga", "/comic", "/novel"],
            vec!["漫画", "イラスト", "小説"],
            47,
            "Japanese manga and illustration sites"
        ),
        
        // 81. 日本音乐
        ClassificationRule::new(
            "japanese_music",
            "日本音乐",
            "Japanese Music",
            vec!["spotify.com/ja", "apple.com/jp/music"],
            vec![
                "music.apple.com", "spotify.com", "youtube.com/music",
                "uta-net.com", "joysound.com", "recochoku.jp",
                "mora.jp", "ototoy.jp"
            ],
            vec!["/music", "/song", "/artist"],
            vec!["音楽", "歌詞", "アーティスト"],
            46,
            "Japanese music streaming and lyrics"
        ),
        
        // 82. 日本工具服务
        ClassificationRule::new(
            "japanese_tools",
            "日本工具",
            "Japanese Tools",
            vec!["yahoo.co.jp", "cookpad", "tabelog"],
            vec![
                "yahoo.co.jp", "cookpad.com", "tabelog.com",
                "gurunavi.com", "hotpepper.jp", "jalan.net",
                "rakuten-travelco.jp", "ekitan.com", "jorudan.co.jp"
            ],
            vec!["/search", "/map", "/tool"],
            vec!["検索", "レシピ", "地図"],
            45,
            "Japanese utility and service sites"
        ),
        
        // 83. 日本教育学习
        ClassificationRule::new(
            "japanese_education",
            "日本教育",
            "Japanese Education",
            vec!["studyplus", "benesse"],
            vec![
                "studyplus.jp", "benesse.jp", "smartstudy.jp",
                "english-speaking.jp", "weblio.jp", "jisho.org",
                "tangorin.com", "takoboto.jp"
            ],
            vec!["/study", "/learn", "/course"],
            vec!["勉強", "学習", "教育"],
            44,
            "Japanese education and learning platforms"
        ),
        
        // 84. 日本二手交易
        ClassificationRule::new(
            "japanese_secondhand",
            "日本二手",
            "Japanese Secondhand",
            vec!["mercari", "yahoo.auction"],
            vec![
                "mercari.com", "auctions.yahoo.co.jp", "rakuma.rakuten.co.jp",
                "jimoty.jp", "aucfan.com", "bookoff.co.jp",
                "hardoff.co.jp", "treasure-f.com"
            ],
            vec!["/auction", "/sell", "/buy"],
            vec!["中古", "オークション", "フリマ"],
            43,
            "Japanese secondhand and auction sites"
        ),
        
        // 85. 日本旅游
        ClassificationRule::new(
            "japanese_travel",
            "日本旅游",
            "Japanese Travel",
            vec!["jalan", "rakuten.travel", "booking.com/ja"],
            vec![
                "jalan.net", "travel.rakuten.co.jp", "booking.com",
                "agoda.com", "じゃらん", "一休.com", "expedia.co.jp",
                "tripadvisor.jp", "ana.co.jp", "jal.co.jp"
            ],
            vec!["/hotel", "/travel", "/flight"],
            vec!["旅行", "ホテル", "予約"],
            42,
            "Japanese travel and booking sites"
        ),
        
        // 86. 日本金融
        ClassificationRule::new(
            "japanese_finance",
            "日本金融",
            "Japanese Finance",
            vec!["rakuten-sec", "sbi", "moneyforward"],
            vec![
                "rakuten-sec.co.jp", "sbisec.co.jp", "moneyforward.com",
                "mufg.jp", "smbc.co.jp", "mizuho-fg.co.jp",
                "japanpost.jp", "paypay.ne.jp", "line-pay.com"
            ],
            vec!["/bank", "/pay", "/finance"],
            vec!["金融", "銀行", "支払い"],
            41,
            "Japanese banking and finance services"
        ),
        
        // 87. 日本求职招聘
        ClassificationRule::new(
            "japanese_jobs",
            "日本求职",
            "Japanese Jobs",
            vec!["rikunabi", "mynavi", "doda"],
            vec![
                "rikunabi.com", "mynavi.jp", "doda.jp",
                "en-japan.com", "bizreach.jp", "wantedly.com",
                "green-japan.com", "indeed.com/jp"
            ],
            vec!["/job", "/career", "/recruit"],
            vec!["求人", "転職", "採用"],
            40,
            "Japanese job hunting and recruitment"
        ),
        
        // 88. 日本健康医疗
        ClassificationRule::new(
            "japanese_health",
            "日本健康",
            "Japanese Health",
            vec!["medicalnote", "caloo"],
            vec![
                "medicalnote.jp", "caloo.jp", "epark.jp",
                "qlife.jp", "doctor-navi.com", "medley.life",
                "minnakenko.jp", "healthcare.omron.co.jp"
            ],
            vec!["/health", "/clinic", "/medical"],
            vec!["健康", "病院", "医療"],
            39,
            "Japanese healthcare and medical sites"
        ),
        
        // 89. 日本综合服务 (原日本服务,优先级降低)
        ClassificationRule::new(
            "japanese_general",
            "日本综合",
            "Japanese General",
            vec![".co.jp", ".jp"],
            vec![
                "fc2.com", "livedoor.com", "goo.ne.jp",
                "excite.co.jp", "biglobe.ne.jp", "nifty.com"
            ],
            vec![],
            vec![],
            38,
            "General Japanese websites and services"
        ),
        
        // 90. 韩国服务
        ClassificationRule::new(
            "korean",
            "韩国服务",
            "Korean Services",
            vec![".co.kr", "naver", "kakao"],
            vec![
                "naver.com", "kakao.com", "daum.net", "coupang.com",
                "11st.co.kr", "gmarket.co.kr", "auction.co.kr",
                "nate.com", "zum.com"
            ],
            vec![],
            vec![],
            51,
            "Korean platforms and services"
        ),
    ]
}

/// Classification statistics
#[derive(Default)]
struct ClassificationStats {
    total_processed: usize,
    total_classified: usize,
    by_category: HashMap<String, usize>,
    unclassified: usize,
}

impl SyncEngine {
    /// Print built-in classification rules
    pub fn print_builtin_rules() {
        let rules = get_builtin_rules();
        
        println!("\n🧠 Built-in Classification Rules");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
        
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
                println!("   Domains: {}{}", domains.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "), more);
            }
            
            if !rule.url_patterns.is_empty() {
                println!("   URL patterns: {}", rule.url_patterns.join(", "));
            }
            
            if !rule.path_patterns.is_empty() {
                let paths: Vec<_> = rule.path_patterns.iter().take(5).collect();
                println!("   Path patterns: {}", paths.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
            }
            
            println!();
        }
        
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
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
    
    /// Smart organize bookmarks using rule engine
    pub async fn smart_organize(
        &mut self,
        browser_names: Option<&str>,
        rules_file: Option<&str>,
        uncategorized_only: bool,
        show_stats: bool,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
        info!("🧠 Starting smart bookmark organization");
        
        // Load rules
        let mut rules = get_builtin_rules();
        
        // Load custom rules if provided
        if let Some(file_path) = rules_file {
            info!("📂 Loading custom rules from: {}", file_path);
            let content = std::fs::read_to_string(file_path)
                .context("Failed to read rules file")?;
            let custom_rules: Vec<ClassificationRule> = serde_json::from_str(&content)
                .context("Failed to parse rules file")?;
            info!("✅ Loaded {} custom rules", custom_rules.len());
            rules.extend(custom_rules);
        }
        
        // Sort rules by priority (higher first)
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        info!("📋 Loaded {} classification rules", rules.len());
        
        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect();
            
            self.adapters.iter()
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
            
            match adapter.read_bookmarks() {
                Ok(mut bookmarks) => {
                    info!("\n📊 {} : Processing...", browser_name);
                    
                    let mut stats = ClassificationStats::default();
                    
                    // Collect bookmarks to classify
                    let mut to_classify: Vec<Bookmark> = Vec::new();
                    if uncategorized_only {
                        // Only collect bookmarks at root level (not in folders)
                        Self::collect_root_bookmarks(&mut bookmarks, &mut to_classify);
                    } else {
                        // Collect all non-folder bookmarks from entire tree
                        Self::collect_all_bookmarks_for_classification(&mut bookmarks, &mut to_classify);
                    }
                    
                    stats.total_processed = to_classify.len();
                    info!("  📖 Found {} bookmarks to classify", to_classify.len());
                    
                    // Classify bookmarks
                    let mut classified: HashMap<String, Vec<Bookmark>> = HashMap::new();
                    let mut unclassified: Vec<Bookmark> = Vec::new();
                    
                    for bookmark in to_classify {
                        let url = bookmark.url.as_ref().map(|s| s.as_str()).unwrap_or("");
                        let title = &bookmark.title;
                        
                        let mut matched = false;
                        for rule in &rules {
                            if rule.matches(url, title) {
                                if verbose {
                                    debug!("  ✓ '{}' -> {} (rule: {})", title, rule.folder_name, rule.name);
                                }
                                classified
                                    .entry(rule.folder_name.clone())
                                    .or_insert_with(Vec::new)
                                    .push(bookmark.clone());
                                *stats.by_category.entry(rule.folder_name.clone()).or_insert(0) += 1;
                                matched = true;
                                break;
                            }
                        }
                        
                        if !matched {
                            unclassified.push(bookmark);
                            stats.unclassified += 1;
                        }
                    }
                    
                    stats.total_classified = stats.total_processed - stats.unclassified;
                    
                    // Create/update folders for classified bookmarks
                    for (folder_name, items) in &classified {
                        let existing_folder = bookmarks.iter_mut()
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
                        
                        info!("  📁 {} : {} bookmarks", folder_name, items.len());
                    }
                    
                    if !unclassified.is_empty() {
                        info!("  ❓ Unclassified: {} bookmarks", unclassified.len());
                        
                        // 🔧 BUG FIX: 将未分类的书签放入"未分类"文件夹，而不是丢弃！
                        let unclassified_folder = bookmarks.iter_mut()
                            .find(|b| b.folder && b.title == "未分类");
                        
                        if let Some(folder) = unclassified_folder {
                            folder.children.extend(unclassified.clone());
                        } else {
                            let new_folder = Bookmark {
                                id: format!("unclassified-folder-{}", chrono::Utc::now().timestamp_millis()),
                                title: "未分类".to_string(),
                                url: None,
                                folder: true,
                                children: unclassified.clone(),
                                date_added: Some(chrono::Utc::now().timestamp_millis()),
                                date_modified: Some(chrono::Utc::now().timestamp_millis()),
                            };
                            bookmarks.push(new_folder);
                        }
                        info!("  📁 Uncategorized : {} bookmarks (preserved)", unclassified.len());
                    }
                    
                    // Show statistics if requested
                    if show_stats {
                        println!("\n📊 Classification Statistics for {}:", browser_name);
                        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                        println!("  Total processed:  {}", stats.total_processed);
                        println!("  Total classified: {} ({:.1}%)", 
                            stats.total_classified,
                            if stats.total_processed > 0 { 
                                stats.total_classified as f64 / stats.total_processed as f64 * 100.0 
                            } else { 0.0 }
                        );
                        println!("  Unclassified:     {}", stats.unclassified);
                        println!("\n  By category:");
                        
                        let mut categories: Vec<_> = stats.by_category.iter().collect();
                        categories.sort_by(|a, b| b.1.cmp(a.1));
                        for (category, count) in categories {
                            println!("    📁 {} : {}", category, count);
                        }
                        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                    }
                    
                    if dry_run {
                        info!("  🏃 Dry run - would classify {} bookmarks into {} folders", 
                              stats.total_classified, classified.len());
                    } else if stats.total_classified > 0 {
                        // Backup first
                        if let Ok(backup_path) = adapter.backup_bookmarks() {
                            info!("  💾 Backup created: {:?}", backup_path);
                        }
                        
                        // Write organized bookmarks
                        match adapter.write_bookmarks(&bookmarks) {
                            Ok(_) => {
                                info!("  ✅ Organization complete");
                            }
                            Err(e) => {
                                error!("  ❌ Failed to write organized bookmarks: {}", e);
                            }
                        }
                    } else {
                        info!("  ✨ No bookmarks to classify");
                    }
                }
                Err(e) => {
                    error!("  ❌ Failed to read bookmarks from {}: {}", browser_name, e);
                }
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
    fn collect_all_bookmarks_for_classification(bookmarks: &mut Vec<Bookmark>, collected: &mut Vec<Bookmark>) {
        // 只保护已分类的文件夹，不保护系统文件夹（需要递归进入）
        let classified_folders = [
            // 分类文件夹 (48个规则对应的文件夹) - 注意：不包含"未分类"
            "登录入口", "社交媒体", "视频流媒体", "开发工具", "购物网站",
            "新闻资讯", "文档参考", "云存储", "邮箱通讯", "金融理财",
            "AI工具", "设计创意", "教育学习", "音乐音频", "游戏娱乐",
            "论坛社区", "管理后台", "API服务", "应用商店", "存档资料",
            "百科知识", "文件托管", "搜索引擎", "NSFW内容", "Discord社群",
            "动漫二次元", "下载资源", "安全隐私", "Linux开源", "微软服务",
            "苹果服务", "谷歌服务", "联邦宇宙", "移动开发", "科学研究",
            "直播平台", "浏览器扩展", "在线工具", "效率工具", "游戏社区",
            "图床托管", "导航目录", "中文平台", "设计素材", "硬件技术",
            "托管项目", "博客站点", "开发者工具",
            "网站主页"
            // 注意："未分类"不在此列表中，允许重新分类
        ];
        
        // 跳过的系统文件夹（不收集其中的书签，但也不递归）
        let skip_folders = ["com.apple.ReadingList", "阅读列表", "History"];
        
        // First pass: recursively process children
        for bookmark in bookmarks.iter_mut() {
            if bookmark.folder {
                if bookmark.title == "未分类" {
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
                    Self::collect_all_bookmarks_for_classification(&mut bookmark.children, collected);
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
}

fn parse_safari_html(html: &str) -> Result<Vec<Bookmark>> {
    use scraper::{Html, Selector};
    
    let document = Html::parse_document(html);
    let link_selector = Selector::parse("a").unwrap();
    
    let mut bookmarks = Vec::new();
    let mut id_counter = 1;
    
    for element in document.select(&link_selector) {
        if let Some(url) = element.value().attr("href") {
            let title = element.text().collect::<String>();
            
            bookmarks.push(Bookmark {
                id: format!("imported-{}", id_counter),
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
            url: url.map(|s| s.to_string()),
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
    fn test_normalize_url() {
        assert_eq!(SyncEngine::normalize_url("https://example.com/"), "https://example.com");
        assert_eq!(SyncEngine::normalize_url("https://example.com"), "https://example.com");
        assert_eq!(SyncEngine::normalize_url("HTTPS://EXAMPLE.COM/"), "https://example.com");
        assert_eq!(SyncEngine::normalize_url("https://example.com#section"), "https://example.com");
        assert_eq!(SyncEngine::normalize_url("  https://example.com/  "), "https://example.com");
    }

    #[test]
    fn test_count_all_bookmarks() {
        let bookmarks = vec![
            create_bookmark("1", "Test1", Some("https://test1.com")),
            create_folder("2", "Folder1", vec![
                create_bookmark("3", "Test2", Some("https://test2.com")),
                create_bookmark("4", "Test3", Some("https://test3.com")),
            ]),
        ];
        assert_eq!(SyncEngine::count_all_bookmarks(&bookmarks), 3);
    }

    #[test]
    fn test_count_all_folders() {
        let bookmarks = vec![
            create_bookmark("1", "Test1", Some("https://test1.com")),
            create_folder("2", "Folder1", vec![
                create_folder("3", "SubFolder", vec![
                    create_bookmark("4", "Test2", Some("https://test2.com")),
                ]),
            ]),
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
            create_folder("2", "NonEmptyFolder", vec![
                create_bookmark("3", "Test", Some("https://test.com")),
            ]),
            create_folder("4", "NestedEmpty", vec![
                create_folder("5", "InnerEmpty", vec![]),
            ]),
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
            create_folder("2", "Folder", vec![
                create_bookmark("3", "Dup2", Some("https://example.com")), // duplicate - deeper, should keep
            ]),
            create_bookmark("4", "Other", Some("https://other.com")),
        ];
        
        SyncEngine::deduplicate_bookmarks_global(&mut bookmarks);
        
        let total = SyncEngine::count_all_bookmarks(&bookmarks);
        assert_eq!(total, 2); // One example.com and one other.com
    }

    #[test]
    fn test_find_folder_by_path() {
        let bookmarks = vec![
            create_folder("1", "Work", vec![
                create_folder("2", "Projects", vec![
                    create_bookmark("3", "Project1", Some("https://project1.com")),
                ]),
            ]),
        ];
        
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
            create_folder("3", "Folder", vec![
                create_bookmark("4", "SubHomepage", Some("https://sub.example.com/")),
                create_bookmark("5", "SubArticle", Some("https://sub.example.com/page")),
            ]),
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
            "Login pages"
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
            "Social media"
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
            "Admin pages"
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
            "Documentation"
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
            "Test"
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
}

impl SyncEngine {
    /// Set hub browsers with Firefox Sync integration
    pub async fn set_hub_browsers_with_firefox_sync(
        &mut self,
        hub_names: &str,
        sync_history: bool,
        sync_reading_list: bool,
        sync_cookies: bool,
        clear_others: bool,
        dry_run: bool,
        verbose: bool,
        firefox_sync_strategy: crate::firefox_sync::SyncStrategy,
    ) -> Result<()> {
        use crate::firefox_sync::{FirefoxSyncHandler, SyncStrategy};
        use crate::firefox_sync_api::FirefoxSyncAPIClient;
        
        // 检测Waterfox profile
        let waterfox_profile = std::path::PathBuf::from(
            std::env::var("HOME")?
        ).join("Library/Application Support/Waterfox/Profiles/ll4fbmm0.default-release");
        
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
            Some(FirefoxSyncHandler::new(&waterfox_profile, firefox_sync_strategy)?)
        } else {
            None
        };
        
        // 在写入前执行Firefox Sync处理
        if let Some(ref handler) = firefox_sync_handler {
            handler.before_write()?;
        }
        
        // 执行正常的同步流程
        self.set_hub_browsers(
            hub_names,
            sync_history,
            sync_reading_list,
            sync_cookies,
            clear_others,
            dry_run,
            verbose
        ).await?;
        
        // 如果使用API策略，上传到云端
        if let Some(ref client) = api_client {
            if !dry_run {
                info!("");
                info!("📤 Uploading bookmarks to Firefox Sync cloud via API...");
                
                // 读取刚写入的书签
                for adapter in &self.adapters {
                    if adapter.browser_type().name().to_lowercase().contains("waterfox") {
                        if let Ok(bookmarks) = adapter.read_bookmarks() {
                            match client.upload_bookmarks(&bookmarks).await {
                                Ok(_) => {
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
                        info!("  {} : {} reading list items", browser_name, reading_list.len());
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
        
        info!("  📚 Merged bookmarks: {} URLs, {} folders", bookmark_count, folder_count);
        info!("  📜 Merged history: {} items", merged_history.len());
        info!("  📖 Merged reading list: {} items", merged_reading_list.len());
        
        if dry_run {
            info!("");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("🏃 Dry Run Mode - Actions that would be performed:");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("  ✅ Write {} bookmarks to Safari", bookmark_count);
            info!("  ✅ Write {} history items to Safari", merged_history.len());
            info!("  ✅ Write {} reading list items to Safari", merged_reading_list.len());
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
                // Backup
                if let Ok(backup_path) = adapter.backup_bookmarks() {
                    info!("  💾 Safari backup: {:?}", backup_path);
                }
                
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
                        info!("  ✅ Wrote {} reading list items to Safari", merged_reading_list.len());
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
                
                // Backup
                if let Ok(backup_path) = adapter.backup_bookmarks() {
                    info!("  💾 {} backup: {:?}", browser_name, backup_path);
                }
                
                // Clear bookmarks (write empty list)
                let empty_bookmarks: Vec<Bookmark> = vec![];
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
            let browser_list: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect();
            
            self.adapters.iter()
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
                    
                    println!("\n📊 {} Bookmark Analysis", browser_name);
                    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                    println!("  Total bookmarks: {}", total);
                    println!("  Total folders: {}", folders);
                    
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
        merge: bool,
        deduplicate: bool,
        verbose: bool,
    ) -> Result<usize> {
        self.export_to_html_with_extra(browser_names, output_path, merge, deduplicate, false, verbose, Vec::new()).await
    }
    
    /// Export all bookmarks with additional bookmarks from external sources
    pub async fn export_to_html_with_extra(
        &self,
        browser_names: Option<&str>,
        output_path: &str,
        merge: bool,
        deduplicate: bool,
        clean_empty: bool,
        verbose: bool,
        extra_bookmarks: Vec<Bookmark>,
    ) -> Result<usize> {
        info!("📤 Exporting bookmarks to HTML...");
        
        // Determine target browsers
        let target_adapters: Vec<_> = if let Some(names) = browser_names {
            let browser_list: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect();
            
            self.adapters.iter()
                .filter(|a| {
                    let name = a.browser_type().name().to_lowercase();
                    let name_normalized = name.replace(' ', "-").replace('_', "-");
                    browser_list.iter().any(|b| {
                        let b_normalized = b.replace(' ', "-").replace('_', "-");
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
        
        if target_adapters.is_empty() {
            anyhow::bail!("No matching browsers found");
        }
        
        info!("🎯 Target browsers:");
        for adapter in &target_adapters {
            info!("  - {}", adapter.browser_type().name());
        }
        
        // Collect bookmarks from all browsers
        let mut all_bookmarks: Vec<Bookmark> = Vec::new();
        let mut browser_stats: Vec<(String, usize)> = Vec::new();
        
        // Add extra bookmarks first (from HTML imports etc.)
        if !extra_bookmarks.is_empty() {
            let extra_count = Self::count_all_bookmarks(&extra_bookmarks);
            info!("  📥 HTML import: {} bookmarks", extra_count);
            browser_stats.push(("HTML Import".to_string(), extra_count));
            all_bookmarks.extend(extra_bookmarks);
        }
        
        for adapter in &target_adapters {
            let browser_name = adapter.browser_type().name();
            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    let count = Self::count_all_bookmarks(&bookmarks);
                    info!("  ✅ {} : {} bookmarks", browser_name, count);
                    browser_stats.push((browser_name.to_string(), count));
                    
                    if merge {
                        // Merge into single list
                        all_bookmarks.extend(bookmarks);
                    } else {
                        // Create a folder for each browser
                        let browser_folder = Bookmark {
                            id: format!("browser-{}", browser_name.to_lowercase().replace(' ', "-")),
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
                    warn!("  ⚠️  {} : read failed - {}", browser_name, e);
                }
            }
        }
        
        let before_dedup = Self::count_all_bookmarks(&all_bookmarks);
        info!("\n📊 Collection complete: {} bookmarks", before_dedup);
        
        // Deduplicate if requested
        if deduplicate {
            info!("🧹 Deduplicating...");
            Self::deduplicate_bookmarks_global(&mut all_bookmarks);
            let after_dedup = Self::count_all_bookmarks(&all_bookmarks);
            let removed = before_dedup.saturating_sub(after_dedup);
            if removed > 0 {
                info!("  ✅ Removed {} duplicate bookmarks", removed);
            }
        }
        
        // Clean empty folders if requested
        if clean_empty {
            info!("🧹 Cleaning empty folders...");
            let mut total_empty_removed = 0;
            loop {
                let removed = Self::cleanup_empty_folders(&mut all_bookmarks);
                if removed == 0 { break; }
                total_empty_removed += removed;
            }
            if total_empty_removed > 0 {
                info!("  ✅ Removed {} empty folders", total_empty_removed);
            }
        }
        
        let final_count = Self::count_all_bookmarks(&all_bookmarks);
        
        // Export to HTML
        let output = if output_path.starts_with("~/") {
            let home = std::env::var("HOME").unwrap_or_default();
            output_path.replacen("~", &home, 1)
        } else {
            output_path.to_string()
        };
        
        export_bookmarks_to_html(&all_bookmarks, &output)?;
        
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
    
    /// Clear bookmarks from specified browsers (use with caution!)
    pub async fn clear_bookmarks(
        &mut self,
        browser_names: &str,
        dry_run: bool,
    ) -> Result<()> {
        info!("🗑️  Clearing browser bookmarks...");
        
        let browser_list: Vec<String> = browser_names
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .collect();
        
        let target_adapters: Vec<_> = self.adapters.iter()
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
            
            // Backup first
            match adapter.backup_bookmarks() {
                Ok(backup_path) => {
                    info!("  💾 {} : backup created {:?}", browser_name, backup_path);
                }
                Err(e) => {
                    warn!("  ⚠️  {} : backup failed - {}", browser_name, e);
                }
            }
            
            // Write empty bookmarks
            match adapter.write_bookmarks(&[]) {
                Ok(_) => {
                    info!("  ✅ {} : cleared", browser_name);
                }
                Err(e) => {
                    error!("  ❌ {} : clear failed - {}", browser_name, e);
                }
            }
        }
        
        Ok(())
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
        let adapter = self.adapters.iter()
            .find(|a| a.browser_type().name().to_lowercase().contains(&browser_lower))
            .ok_or_else(|| anyhow::anyhow!("Browser not found: {}", browser_name))?;
        
        let browser_type_name = adapter.browser_type().name();
        
        // Find backup file
        let backup_path = if let Some(file) = backup_file {
            std::path::PathBuf::from(file)
        } else {
            // Find latest backup
            let bookmark_path = adapter.detect_bookmark_path()?;
            let backup_path = bookmark_path.with_extension("sqlite.backup");
            
            if !backup_path.exists() {
                // Try other backup extensions
                let backup_path2 = bookmark_path.with_extension("sqlite.cloud_reset_backup");
                if backup_path2.exists() {
                    backup_path2
                } else {
                    anyhow::bail!("未找到备份文件。请使用 -f 指定备份文件路径");
                }
            } else {
                backup_path
            }
        };
        
        if !backup_path.exists() {
            anyhow::bail!("备份文件不存在: {:?}", backup_path);
        }
        
        info!("📂 备份文件: {:?}", backup_path);
        
        // Get current bookmark path
        let current_path = adapter.detect_bookmark_path()?;
        
        // Create a backup of current state before restore
        let pre_restore_backup = current_path.with_extension("sqlite.pre_restore_backup");
        if current_path.exists() {
            std::fs::copy(&current_path, &pre_restore_backup)?;
            info!("💾 当前状态已备份到: {:?}", pre_restore_backup);
        }
        
        // Restore
        std::fs::copy(&backup_path, &current_path)?;
        
        // Verify
        match adapter.read_bookmarks() {
            Ok(bookmarks) => {
                let count = Self::count_all_bookmarks(&bookmarks);
                info!("✅ 恢复成功! {} 现在有 {} 个书签", browser_type_name, count);
            }
            Err(e) => {
                warn!("⚠️  恢复后验证失败: {}", e);
                warn!("   尝试恢复原状态...");
                if pre_restore_backup.exists() {
                    std::fs::copy(&pre_restore_backup, &current_path)?;
                }
                anyhow::bail!("恢复失败");
            }
        }
        
        Ok(())
    }
}

/// Create comprehensive master backup from all browser data
pub async fn create_master_backup(output_dir: &str, include_full: bool) -> Result<()> {
    use std::collections::HashMap as StdHashMap;
    
    info!("📦 创建主备份...");
    
    let output_path = if output_dir.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        output_dir.replacen("~", &home, 1)
    } else {
        output_dir.to_string()
    };
    std::fs::create_dir_all(&output_path)?;
    
    let mut all_bookmarks = Vec::new();
    let mut source_stats = StdHashMap::new();
    
    // 收集所有浏览器数据
    let browsers = [
        BrowserType::Safari, 
        BrowserType::Chrome, 
        BrowserType::Waterfox, 
        BrowserType::Brave,
        BrowserType::BraveNightly,
    ];
    
    for browser in browsers {
        let adapters = crate::browsers::get_all_adapters();
        if let Some(adapter) = adapters.iter().find(|a| a.browser_type() == browser) {
            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    let count = count_bookmarks_recursive(&bookmarks);
                    if count > 0 {
                        info!("  📱 {}: {} 书签", browser.name(), count);
                        source_stats.insert(browser.name().to_string(), count);
                        
                        collect_urls_recursive(&bookmarks, browser.name(), &mut all_bookmarks);
                    }
                }
                Err(_) => continue,
            }
        }
    }
    
    info!("📊 总计收集: {} 条记录", all_bookmarks.len());
    
    // 去重
    let mut unique_urls = StdHashMap::new();
    for bookmark in &all_bookmarks {
        let url = bookmark["url"].as_str().unwrap_or("").to_lowercase();
        let url_key = url.trim_end_matches('/');
        if !url_key.is_empty() {
            unique_urls.entry(url_key.to_string()).or_insert(bookmark.clone());
        }
    }
    
    info!("📊 唯一URL: {} 个", unique_urls.len());
    
    // 保存文件
    if include_full {
        let full_path = format!("{}/all_bookmarks_full.json", output_path);
        std::fs::write(&full_path, serde_json::to_string_pretty(&all_bookmarks)?)?;
        info!("✅ 完整数据: {}", full_path);
    }
    
    let unique_path = format!("{}/unique_bookmarks.json", output_path);
    let unique_list: Vec<_> = unique_urls.into_values().collect();
    std::fs::write(&unique_path, serde_json::to_string_pretty(&unique_list)?)?;
    info!("✅ 唯一URL: {}", unique_path);
    
    let stats_path = format!("{}/sources_stats.json", output_path);
    std::fs::write(&stats_path, serde_json::to_string_pretty(&source_stats)?)?;
    info!("✅ 来源统计: {}", stats_path);
    
    let urls_path = format!("{}/all_urls.txt", output_path);
    let mut urls: Vec<_> = unique_list.iter()
        .filter_map(|b| b["url"].as_str())
        .collect();
    urls.sort();
    std::fs::write(&urls_path, urls.join("\n"))?;
    info!("✅ URL列表: {}", urls_path);
    
    info!("\n✅ 主备份创建完成: {}", output_path);
    
    Ok(())
}

fn count_bookmarks_recursive(bookmarks: &[Bookmark]) -> usize {
    let mut count = 0;
    for bookmark in bookmarks {
        if bookmark.url.is_some() {
            count += 1;
        }
        count += count_bookmarks_recursive(&bookmark.children);
    }
    count
}

fn collect_urls_recursive(bookmarks: &[Bookmark], source: &str, result: &mut Vec<serde_json::Value>) {
    for bookmark in bookmarks {
        if let Some(url) = &bookmark.url {
            result.push(serde_json::json!({
                "url": url,
                "title": bookmark.title,
                "source": source
            }));
        }
        collect_urls_recursive(&bookmark.children, source, result);
    }
}

/// Export bookmarks to Netscape HTML format (standard bookmark format)
pub fn export_bookmarks_to_html(bookmarks: &[Bookmark], output_path: &str) -> Result<()> {
    use std::io::Write;
    
    let mut file = std::fs::File::create(output_path)?;
    
    // Write HTML header
    writeln!(file, "<!DOCTYPE NETSCAPE-Bookmark-file-1>")?;
    writeln!(file, "<!-- This is an automatically generated file.")?;
    writeln!(file, "     It will be read and overwritten.")?;
    writeln!(file, "     DO NOT EDIT! -->")?;
    writeln!(file, "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">")?;
    writeln!(file, "<TITLE>Bookmarks</TITLE>")?;
    writeln!(file, "<H1>Bookmarks</H1>")?;
    writeln!(file, "<DL><p>")?;
    
    // Write bookmarks recursively
    write_bookmarks_html_recursive(&mut file, bookmarks, 1)?;
    
    writeln!(file, "</DL><p>")?;
    
    Ok(())
}

fn write_bookmarks_html_recursive<W: Write>(writer: &mut W, bookmarks: &[Bookmark], indent: usize) -> Result<()> {
    let indent_str = "    ".repeat(indent);
    
    for bookmark in bookmarks {
        if bookmark.folder {
            // Write folder
            let add_date = bookmark.date_added.unwrap_or(0) / 1000; // Convert to seconds
            writeln!(writer, "{}<DT><H3 ADD_DATE=\"{}\">{}</H3>", 
                indent_str, add_date, html_escape(&bookmark.title))?;
            writeln!(writer, "{}<DL><p>", indent_str)?;
            write_bookmarks_html_recursive(writer, &bookmark.children, indent + 1)?;
            writeln!(writer, "{}</DL><p>", indent_str)?;
        } else if let Some(url) = &bookmark.url {
            // Write bookmark
            let add_date = bookmark.date_added.unwrap_or(0) / 1000;
            writeln!(writer, "{}<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>", 
                indent_str, html_escape(url), add_date, html_escape(&bookmark.title))?;
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

/// Import bookmarks from HTML file
pub fn import_bookmarks_from_html(html_path: &str) -> Result<Vec<Bookmark>> {
    let content = std::fs::read_to_string(html_path)?;
    parse_html_bookmarks(&content)
}

fn parse_html_bookmarks(html: &str) -> Result<Vec<Bookmark>> {
    let mut bookmarks = Vec::new();
    let mut id_counter = 0u64;
    
    // Simple flat parsing - just extract all bookmarks
    for line in html.lines() {
        let trimmed = line.trim();
        
        // Parse bookmark links
        if (trimmed.contains("<DT><A") || trimmed.contains("<dt><a")) && 
           (trimmed.contains("HREF=") || trimmed.contains("href=")) {
            if let Some((url, title)) = extract_bookmark_info(trimmed) {
                id_counter += 1;
                let bookmark = Bookmark {
                    id: format!("imported-{}", id_counter),
                    title,
                    url: Some(url),
                    folder: false,
                    children: Vec::new(),
                    date_added: Some(chrono::Utc::now().timestamp_millis()),
                    date_modified: None,
                };
                bookmarks.push(bookmark);
            }
        }
    }
    
    info!("📖 HTML解析完成: {} 书签", bookmarks.len());
    Ok(bookmarks)
}

fn extract_tag_content(line: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{}", tag);
    let end_tag = format!("</{}>", tag);
    
    if let Some(start_idx) = line.to_lowercase().find(&start_tag.to_lowercase()) {
        if let Some(content_start) = line[start_idx..].find('>') {
            let content_start = start_idx + content_start + 1;
            if let Some(end_idx) = line.to_lowercase().find(&end_tag.to_lowercase()) {
                let content = &line[content_start..end_idx];
                return Some(html_unescape(content));
            }
        }
    }
    None
}

fn extract_bookmark_info(line: &str) -> Option<(String, String)> {
    // Extract URL from HREF="..."
    let href_start = line.to_lowercase().find("href=\"")?;
    let url_start = href_start + 6;
    let url_end = line[url_start..].find('"')? + url_start;
    let url = html_unescape(&line[url_start..url_end]);
    
    // Extract title (content between > and </A>)
    let title_start = line[url_end..].find('>')? + url_end + 1;
    let title_end = line.to_lowercase().find("</a>")?;
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
