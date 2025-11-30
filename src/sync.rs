use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
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

pub struct SyncEngine {
    adapters: Vec<Box<dyn BrowserAdapter + Send + Sync>>,
}

impl SyncEngine {
    pub fn new() -> Result<Self> {
        let adapters = get_all_adapters();
        Ok(Self { adapters })
    }

    pub async fn sync(&mut self, dry_run: bool, verbose: bool) -> Result<()> {
        info!("🔍 Phase 1: Pre-sync validation");
        self.pre_sync_validation()?;

        info!("📖 Phase 2: Reading bookmarks from all browsers");
        let mut browser_bookmarks = HashMap::new();
        
        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();
            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    info!("✅ Read {} bookmarks from {}", bookmarks.len(), browser_type.name());
                    browser_bookmarks.insert(browser_type, bookmarks);
                }
                Err(e) => {
                    warn!("⚠️  Failed to read bookmarks from {}: {}", browser_type.name(), e);
                }
            }
        }

        if browser_bookmarks.is_empty() {
            error!("❌ No bookmarks could be read from any browser");
            anyhow::bail!("No bookmarks available for synchronization");
        }

        info!("🔄 Phase 3: Merging bookmarks");
        let merged = self.merge_bookmarks(&browser_bookmarks, verbose)?;
        info!("📊 Merged result: {} unique bookmarks", merged.len());

        if dry_run {
            info!("🏃 Dry run mode - no changes will be made");
            self.print_sync_preview(&browser_bookmarks, &merged);
            return Ok(());
        }

        info!("💾 Phase 4: Creating backups");
        for adapter in &self.adapters {
            match adapter.backup_bookmarks() {
                Ok(backup_path) => {
                    info!("✅ Backup created for {}: {:?}", adapter.browser_type().name(), backup_path);
                }
                Err(e) => {
                    warn!("⚠️  Failed to backup {}: {}", adapter.browser_type().name(), e);
                }
            }
        }

        info!("✍️  Phase 5: Writing merged bookmarks");
        for adapter in &self.adapters {
            let browser_type = adapter.browser_type();
            match adapter.write_bookmarks(&merged) {
                Ok(_) => {
                    info!("✅ Wrote bookmarks to {}", browser_type.name());
                }
                Err(e) => {
                    error!("❌ Failed to write bookmarks to {}: {}", browser_type.name(), e);
                }
            }
        }

        info!("🔍 Phase 6: Post-sync validation");
        self.post_sync_validation(&merged)?;

        Ok(())
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

    fn post_sync_validation(&self, _expected: &[Bookmark]) -> Result<()> {
        let mut validation_passed = true;

        for adapter in &self.adapters {
            match adapter.read_bookmarks() {
                Ok(bookmarks) => {
                    if adapter.validate_bookmarks(&bookmarks)? {
                        debug!("✅ {} validation passed", adapter.browser_type().name());
                    } else {
                        warn!("⚠️  {} validation failed", adapter.browser_type().name());
                        validation_passed = false;
                    }
                }
                Err(e) => {
                    warn!("⚠️  Could not validate {}: {}", adapter.browser_type().name(), e);
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
        
        // Deduplicate bookmarks by URL within the tree structure
        let before_count = Self::count_all_bookmarks(&merged);
        
        // Global deduplication - track all URLs across entire tree with smart selection
        Self::deduplicate_bookmarks_global(&mut merged);
        
        let after_count = Self::count_all_bookmarks(&merged);
        
        if before_count != after_count {
            info!("🔄 Deduplicated: {} → {} URLs (removed {} duplicates)", 
                before_count, after_count, before_count - after_count);
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
                        info!("  📁 Moved {} homepage bookmarks to root \"网站主页\" folder", moved_count);
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
}
