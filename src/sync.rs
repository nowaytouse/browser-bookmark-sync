use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use tracing::{info, warn, error, debug};
use sha2::{Sha256, Digest};

use crate::browsers::{Bookmark, BrowserAdapter, BrowserType, get_all_adapters};
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

    fn post_sync_validation(&self, expected: &[Bookmark]) -> Result<()> {
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
}
