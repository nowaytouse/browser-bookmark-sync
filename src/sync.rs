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
        let mut merged = Vec::new();
        let mut seen_urls = HashSet::new();

        for (browser, bookmarks) in browser_bookmarks {
            if verbose {
                debug!("Processing {} bookmarks from {}", bookmarks.len(), browser.name());
            }

            for bookmark in bookmarks {
                if bookmark.folder {
                    // Always include folders
                    merged.push(bookmark.clone());
                } else if let Some(url) = &bookmark.url {
                    // Deduplicate by URL
                    let url_hash = self.hash_url(url);
                    if seen_urls.insert(url_hash) {
                        merged.push(bookmark.clone());
                    } else if verbose {
                        debug!("Skipping duplicate URL: {}", url);
                    }
                }
            }
        }

        // Sort by title for consistency
        merged.sort_by(|a, b| a.title.cmp(&b.title));

        Ok(merged)
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
            println!("  {} {} bookmarks", browser.name(), bookmarks.len());
        }
        
        println!("  ─────────────────────────────────────────");
        println!("  Merged: {} unique bookmarks", merged.len());
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
