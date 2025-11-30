use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use tracing::{info, warn, error, debug};
use sha2::{Sha256, Digest};

use crate::browsers::{Bookmark, BrowserAdapter, BrowserType, get_all_adapters, HistoryItem, ReadingListItem, Cookie};
use crate::validator::ValidationReport;

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
        let merged = if let Some(browser) = best_browser {
            let bookmarks = browser_bookmarks.get(&browser).cloned().unwrap_or_default();
            let url_count = Self::count_all_bookmarks(&bookmarks);
            let folder_count = Self::count_all_folders(&bookmarks);
            info!("📚 Using {} as base ({} URLs, {} folders)", browser.name(), url_count, folder_count);
            bookmarks
        } else {
            Vec::new()
        };
        
        Ok(merged)
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

    /// Migrate all browser data to hub browsers
    /// This is the main migration function that:
    /// 1. Reads ALL data from ALL browsers (bookmarks, history, cookies)
    /// 2. Merges and deduplicates
    /// 3. Writes to hub browsers only
    /// 4. Optionally clears non-hub browsers
    pub async fn migrate_to_hubs(
        &mut self,
        hub_names: &str,
        sync_history: bool,
        sync_cookies: bool,
        clear_others: bool,
        dry_run: bool,
        verbose: bool,
    ) -> Result<()> {
        // Reading list is always included (it's part of bookmarks for Chromium browsers)
        let sync_reading_list = true;
        self.set_hub_browsers(hub_names, sync_history, sync_reading_list, sync_cookies, clear_others, dry_run, verbose).await
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
