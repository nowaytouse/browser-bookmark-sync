use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

use browser_bookmark_sync::{
    browsers, data_types, expand_path, gui,
    health::{
        reporting::ReportExporter, HealthEngine, HealthEngineOptions, HealthVerdict, ProbeTarget,
    },
    sync::{self, SyncEngine},
    sync_flags::SyncFlags,
    url_checker,
};

#[derive(Parser)]
#[command(name = "bsync")]
#[command(about = "Cross-browser bookmark sync tool - merge, deduplicate, export")]
#[command(version)]
#[command(after_help = "EXAMPLES:
    bsync list                              # List detected browsers
    bsync export -d --merge                 # Export all, deduplicated, merged
    bsync export -b safari --reading-list   # Export Safari with reading list
    bsync analyze                           # Analyze bookmarks for issues
    bsync organize --dry-run                # Preview smart organization")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect and audit installed browsers and profiles without writing HTML
    Inventory {
        /// Include all profiles of each browser
        #[arg(short = 'a', long, default_value = "true")]
        all_profiles: bool,
    },

    /// Collect original un-deduplicated bookmark tree and export inventory
    Collect {
        /// Output directory for original HTML and inventory
        #[arg(short, long, default_value = "./bookmark-run")]
        output_dir: String,

        /// Preserve original source tree hierarchy
        #[arg(long, default_value = "true")]
        preserve_source_tree: bool,
    },

    /// Perform exact deduplication over harvested inventory or bookmark tree
    Dedupe {
        /// Output directory for deduplicated HTML and audit reports
        #[arg(short, long, default_value = "./bookmark-run")]
        output_dir: String,

        /// Deduplication mode (exact)
        #[arg(long, default_value = "exact")]
        mode: String,
    },

    /// List detected browsers and bookmark counts
    #[command(alias = "l", alias = "ls")]
    List,

    /// Export browser data to HTML or JSON file (safe, non-destructive)
    #[command(alias = "e", alias = "exp")]
    Export {
        /// Output file path
        #[arg(short, long, default_value = "~/Downloads/all_bookmarks.html")]
        output: String,

        /// Include bookmarks from all user profiles of each browser (default: true)
        #[arg(short = 'a', long, default_value = "true")]
        all_profiles: bool,

        /// Source browsers (comma-separated, or 'all')
        #[arg(short, long, default_value = "all")]
        browsers: String,

        /// Include bookmarks (default: true)
        #[arg(long, default_value = "true")]
        bookmarks: bool,

        /// Include browsing history
        #[arg(long)]
        history: bool,

        /// Include reading list (Safari, Firefox)
        #[arg(short = 'r', long)]
        reading_list: bool,

        /// Include cookies (⚠️  affects sessions)
        #[arg(long)]
        cookies: bool,

        /// Include passwords (⚠️  SECURITY RISK - BLOCKED)
        #[arg(long)]
        passwords: bool,

        /// Include extensions (⚠️  NOT SUPPORTED - BLOCKED)
        #[arg(long)]
        extensions: bool,

        /// Days of history to export (default: 30, 0 = all)
        #[arg(long, default_value = "30")]
        history_days: i32,

        /// Remove duplicate bookmarks/URLs (default: true)
        #[arg(short, long, default_value = "true")]
        deduplicate: bool,

        /// Merge into flat structure (no browser folders)
        #[arg(short, long)]
        merge: bool,

        /// Remove empty folders (default: true)
        #[arg(long, default_value = "true")]
        clean: bool,

        /// Import from existing HTML files, SQLite databases, or external profile directories
        #[arg(long, num_args = 0..)]
        include: Vec<String>,

        /// Clear source browsers after export (⚠️  DANGEROUS!)
        #[arg(long)]
        clear_after: bool,

        /// Enable unsafe database writes (required for `clear_after`)
        #[arg(long)]
        unsafe_write: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Only export bookmarks from specific folder name (e.g., "👀临时" or "Temp")
        /// Searches all browsers for folders matching this name
        #[arg(short = 'f', long)]
        folder: Option<String>,

        /// Flatten export: remove browser root folders (Waterfox, Brave, etc.)
        /// Prevents nested "Imported > Waterfox > Brave" structure when importing (default: true)
        #[arg(long, default_value = "true")]
        flat: bool,

        /// Custom wrap folder name (default: "📁镜像文件夹")
        #[arg(short = 'w', long)]
        wrap: Option<String>,

        /// Disable wrapping all bookmarks in root folder (default: wrapping is ON)
        #[arg(long)]
        no_wrap: bool,

        /// Update existing HTML file with new bookmarks (incremental export)
        /// Skips bookmarks that already exist in the target file
        #[arg(short = 'u', long)]
        update: Option<String>,

        /// Copy exported bookmark HTML to a copy path and perform validity checking
        #[arg(long)]
        validate_copy: Option<String>,

        /// Enable content-level validity checking (checks specific page/video URLs for 404/410 instead of just apex domain)
        #[arg(long)]
        validate_content: bool,

        /// Concurrency limit for validity checking (default: 10, max: 10)
        #[arg(long, default_value = "10")]
        validate_concurrency: usize,

        /// Limit number of URLs to check during copy validation (default: 0 = all)
        #[arg(long, default_value = "0")]
        validate_limit: usize,
    },

    /// Analyze bookmarks (duplicates, empty folders, NSFW)
    #[command(alias = "a")]
    Analyze {
        /// Target browsers
        #[arg(short, long)]
        browsers: Option<String>,
    },

    /// Smart organize bookmarks by URL patterns
    #[command(alias = "org", alias = "o")]
    Organize {
        /// Target browsers (ignored if --file is specified)
        #[arg(short, long)]
        browsers: Option<String>,

        /// Input bookmark files (HTML/JSON) - organize from exported files instead of browser
        /// Multiple files will be merged before organizing
        #[arg(short, long, num_args = 1..)]
        file: Option<Vec<String>>,

        /// Output file path (auto-generated if not specified)
        #[arg(short, long)]
        output: Option<String>,

        /// Custom rules file (JSON)
        #[arg(short, long)]
        rules: Option<String>,

        /// Remove duplicate bookmarks by URL (default: true)
        #[arg(short, long, default_value = "true")]
        deduplicate: bool,

        /// Remove empty folders after organizing (default: true)
        #[arg(long, default_value = "true")]
        clean: bool,

        /// Show statistics
        #[arg(short, long)]
        stats: bool,

        /// Preview only, no changes
        #[arg(long)]
        dry_run: bool,

        /// Verbose output
        #[arg(short = 'V', long)]
        verbose: bool,

        /// Classifier backend: auto (try Ollama then fallback), ollama, or rules
        #[arg(long, default_value = "auto")]
        classifier: String,

        /// Ollama model name
        #[arg(long, default_value = "qwen2.5:7b")]
        ollama_model: String,

        /// Ollama API base URL
        #[arg(long, default_value = "http://127.0.0.1:11434")]
        ollama_url: String,

        /// Number of bookmarks per Ollama request
        #[arg(long, default_value = "20")]
        ollama_batch_size: usize,
    },

    /// Validate bookmark integrity
    #[command(alias = "v")]
    Validate {
        /// Detailed report
        #[arg(short, long)]
        detailed: bool,
    },

    /// Sync browsing history between browsers
    #[command(alias = "hist")]
    History {
        /// Target browsers
        #[arg(short, long, default_value = "waterfox,brave-nightly")]
        browsers: String,

        /// Days to sync
        #[arg(short, long, default_value = "30")]
        days: i32,

        /// Preview only
        #[arg(long)]
        dry_run: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show available classification rules
    Rules,

    /// Check bookmark URL validity (dual-network validation)
    #[command(alias = "c", alias = "chk")]
    Check {
        /// Input bookmark file (HTML) - check from exported file instead of browser
        #[arg(short, long)]
        file: Option<String>,

        /// Output file path (required when using --file, saves valid bookmarks)
        #[arg(short, long)]
        output: Option<String>,

        /// Proxy server URL (e.g., <http://127.0.0.1:7890>)
        #[arg(short, long)]
        proxy: Option<String>,

        /// Request timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,

        /// Number of concurrent requests (max 5 to prevent system overload)
        #[arg(short, long, default_value = "5")]
        concurrency: usize,

        /// Delete confirmed invalid bookmarks
        #[arg(long)]
        delete: bool,

        /// Preview mode, no actual changes
        #[arg(long)]
        dry_run: bool,

        /// Verbose output (show HTTP status codes)
        #[arg(short, long)]
        verbose: bool,

        /// Target browsers (comma-separated, or 'all') - ignored if --file is specified
        #[arg(short, long, default_value = "all")]
        browsers: String,

        /// Limit number of URLs to check (default: 100, 0 = no limit - USE WITH CAUTION!)
        #[arg(short = 'L', long, default_value = "100")]
        limit: usize,

        /// Export invalid bookmarks to HTML file before deletion
        #[arg(short = 'e', long)]
        export_invalid: Option<String>,

        /// Export all results to directory (valid.html, invalid.html, uncertain.html, skipped.html)
        #[arg(long)]
        export_dir: Option<String>,

        /// Keep empty folders after deletion (default: remove empty folders)
        #[arg(long)]
        keep_empty: bool,

        /// Enable slow conservative probing mode
        #[arg(long)]
        slow: bool,

        /// Path to SQLite health history database
        #[arg(long)]
        history_db: Option<String>,

        /// Secondary probe backends (comma separated, e.g., lychee,httpx)
        #[arg(long)]
        secondary: Option<String>,
    },

    /// Manage multi-run bookmark health history, audit tools, and quarantine reports
    Health {
        #[command(subcommand)]
        command: browser_bookmark_sync::health::HealthCommands,
    },

    /// Create full backup of all browser data
    Backup {
        /// Output directory
        #[arg(short, long, default_value = "~/Desktop/BookmarkBackup")]
        output: String,
    },

    /// Export sensitive browser data (passwords, cookies, downloads)
    #[command(alias = "data")]
    ExportData {
        /// Browser to export from
        #[arg(short, long, default_value = "chrome")]
        browser: String,

        /// Export passwords
        #[arg(long)]
        passwords: bool,

        /// Export cookies
        #[arg(long)]
        cookies: bool,

        /// Export downloads
        #[arg(long)]
        downloads: bool,

        /// Export all data types
        #[arg(short, long)]
        all: bool,

        /// Output directory
        #[arg(short, long, default_value = "~/Desktop/browser-export")]
        output: String,

        /// Output format: csv, json, netscape (cookies only)
        #[arg(short, long, default_value = "csv")]
        format: String,
    },

    /// Run full bookmark processing pipeline (Export -> Organize -> Check -> Merge)
    #[command(alias = "full-process", alias = "process", alias = "pipe")]
    Pipeline {
        /// Output directory path
        #[arg(short, long, default_value = "~/Downloads/pipeline_results")]
        output_dir: String,

        /// Source browsers (comma-separated, or 'all')
        #[arg(short, long, default_value = "all")]
        browsers: String,

        /// Include extra HTML files, SQLite databases, or external profile directories
        #[arg(short, long, num_args = 0..)]
        include: Vec<String>,

        /// Proxy URL for dead link checking
        #[arg(short, long)]
        proxy: Option<String>,

        /// Check limit (0 = check all)
        #[arg(short, long, default_value = "0")]
        limit: usize,

        /// Concurrent check tasks
        #[arg(short = 'c', long, default_value = "10")]
        concurrency: usize,

        /// HTTP timeout in seconds
        #[arg(short, long, default_value = "15")]
        timeout: u64,
    },

    /// Merge check result HTML files into a single unified HTML file
    #[command(alias = "mr")]
    MergeResults {
        /// Result directory containing valid.html, invalid.html, etc.
        #[arg(short, long)]
        dir: String,

        /// Output file path
        #[arg(short, long, default_value = "~/Downloads/merged_results.html")]
        output: String,
    },

    /// Run full Waterfox bookmark cleanup process
    #[command(alias = "wc")]
    WaterfoxCleanup {
        /// Output file path
        #[arg(short, long, default_value = "~/Downloads/waterfox_cleaned.html")]
        output: String,
    },

    /// Launch local web GUI in browser (default: <http://127.0.0.1:7890>)
    #[command(alias = "ui", alias = "dashboard")]
    Gui {
        /// Port number for the local web server
        #[arg(short, long, default_value = "7890")]
        port: u16,
    },
}

fn print_sync_warning() {
    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    warn!("⚠️  WARNING: This operation modifies browser data!");
    warn!("   If browser sync is enabled, changes may cause conflicts.");
    warn!("   RECOMMENDED: Use 'export' instead and import manually.");
    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Inventory { .. } => {
            use sync::collector::BookmarkCollector;
            let installations = BookmarkCollector::discover_installations();
            info!(
                "🔍 Discovered {} browser installation(s)",
                installations.len()
            );
            for inst in &installations {
                let profiles = BookmarkCollector::discover_profiles(inst);
                info!(
                    "  🌐 {} ({} profile(s)): {:?}",
                    inst.name,
                    profiles.len(),
                    profiles.iter().map(|p| &p.name).collect::<Vec<_>>()
                );
            }
        }

        Commands::Collect { output_dir, .. } => {
            use sync::collector::BookmarkCollector;
            use sync::exporter::ReportExporter;
            let out_path = std::path::PathBuf::from(&output_dir);
            std::fs::create_dir_all(&out_path)?;
            let (tree, inventory, manifest) = BookmarkCollector::collect_all(&out_path)?;

            let orig_html_path = out_path.join("bookmarks-original.html");
            ReportExporter::export_original_html(&tree, &orig_html_path)?;
            ReportExporter::write_sidecar_reports(&out_path, &inventory, &[], &[], &manifest)?;

            info!(
                "✅ Collection complete! Exported original HTML: {:?}",
                orig_html_path
            );
            info!("✅ Sidecar inventory & manifest written to {:?}", out_path);
        }

        Commands::Dedupe { output_dir, .. } => {
            use sync::collector::BookmarkCollector;
            use sync::dedupe::DedupeEngine;
            use sync::exporter::ReportExporter;
            let out_path = std::path::PathBuf::from(&output_dir);
            std::fs::create_dir_all(&out_path)?;

            let (tree, inventory, mut manifest) = BookmarkCollector::collect_all(&out_path)?;
            let (canonicals, reports, csv_records, total_raw, total_dups) =
                DedupeEngine::deduplicate_tree(&tree, &[]);

            manifest.total_canonical_bookmarks = canonicals.len();
            manifest.total_removed_duplicates = total_dups;
            manifest.invariant_satisfied = total_raw == canonicals.len() + total_dups;

            let dedup_html_path = out_path.join("bookmarks-deduplicated.html");
            ReportExporter::export_deduplicated_html(&canonicals, &dedup_html_path)?;
            ReportExporter::write_sidecar_reports(
                &out_path,
                &inventory,
                &reports,
                &csv_records,
                &manifest,
            )?;

            info!("✅ Deduplication complete! Output: {:?}", dedup_html_path);
            info!(
                "📊 Total raw: {}, Canonical kept: {}, Duplicates merged: {}",
                total_raw,
                canonicals.len(),
                total_dups
            );
            info!(
                "🔒 Invariant check (Raw == Canonical + Merged): {}",
                manifest.invariant_satisfied
            );
        }
        Commands::List => {
            let engine = SyncEngine::new()?;
            engine.list_browsers()?;
        }

        Commands::Export {
            output,
            all_profiles,
            browsers,
            bookmarks,
            history,
            reading_list,
            cookies,
            history_days,
            deduplicate,
            merge,
            clean,
            include,
            clear_after,
            unsafe_write,
            passwords,
            extensions,
            verbose,
            folder,
            flat,
            wrap,
            no_wrap,
            update,
            validate_copy,
            validate_content,
            validate_concurrency,
            validate_limit,
        } => {
            // Create sync flags from arguments
            let sync_flags = SyncFlags {
                bookmarks,
                history,
                reading_list,
                cookies,
                passwords,
                extensions,
                history_days: if history_days > 0 {
                    Some(history_days)
                } else {
                    None
                },
                deduplicate,
                merge,
                verbose,
            };

            // Validate flags
            if let Err(e) = sync_flags.validate() {
                error!("{}", e);
                return Ok(());
            }

            info!("📤 Exporting browser data");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("Output: {}", output);
            info!("Source: {}", browsers);
            info!("Data Types: {}", sync_flags.description());
            if deduplicate {
                info!("  ✓ Deduplicate");
            }
            if merge {
                info!("  ✓ Merge (flat)");
            }
            if clean {
                info!("  ✓ Clean empty folders");
            }
            if clear_after {
                warn!("  ⚠️  Clear after export");
            }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            let mut engine = SyncEngine::new()?;

            // Import from existing HTML files, SQLite files, or external profile directories
            let mut extra_bookmarks: Vec<browsers::Bookmark> = Vec::new();
            let mut external_source_reports: Vec<serde_json::Value> = Vec::new();
            for source_path in &include {
                let expanded = expand_path(source_path);
                info!("📥 导入外部数据源: {}", expanded);
                match sync::import_bookmarks_from_source_with_report(&expanded) {
                    Ok((bookmarks, report)) => {
                        let count: usize = bookmarks.iter().map(count_tree).sum();
                        info!("   ✅ 成功从 '{}' 导入 {} 条书签节点", expanded, count);
                        external_source_reports.push(report);
                        extra_bookmarks.extend(bookmarks);
                    }
                    Err(e) => {
                        error!("❌ 无法从外部数据源 '{}' 导入书签: {}", expanded, e);
                        return Err(e.context(format!("从源 '{expanded}' 导入书签失败")));
                    }
                }
            }

            // Include Safari reading list if requested
            if reading_list {
                info!("📖 Reading Safari reading list...");
                match engine.get_safari_reading_list() {
                    Ok(items) if !items.is_empty() => {
                        info!("   {} items found", items.len());
                        let reading_folder = browsers::Bookmark {
                            id: "reading-list".to_string(),
                            title: "Reading List".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .map(|item| browsers::Bookmark {
                                    id: format!("rl-{}", item.url.len()),
                                    title: item.title,
                                    url: Some(item.url),
                                    folder: false,
                                    children: vec![],
                                    date_added: item.date_added,
                                    date_modified: None,
                                })
                                .collect(),
                            date_added: Some(chrono::Utc::now().timestamp_millis()),
                            date_modified: None,
                        };
                        extra_bookmarks.push(reading_folder);
                    }
                    Ok(_) => info!("   No reading list items"),
                    Err(e) => warn!("   Failed to read: {}", e),
                }
            }

            // Include History if requested
            if history {
                info!("📜 Reading history...");
                match engine.get_all_history(sync_flags.history_days) {
                    Ok(items) if !items.is_empty() => {
                        info!("   {} history items found", items.len());
                        let history_folder = browsers::Bookmark {
                            id: "history".to_string(),
                            title: "History".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .enumerate()
                                .map(|(i, item)| browsers::Bookmark {
                                    id: format!("hist-{i}"),
                                    title: item.title.unwrap_or_default(),
                                    url: Some(item.url),
                                    folder: false,
                                    children: vec![],
                                    date_added: item.last_visit,
                                    date_modified: None,
                                })
                                .collect(),
                            date_added: Some(chrono::Utc::now().timestamp_millis()),
                            date_modified: None,
                        };
                        extra_bookmarks.push(history_folder);
                    }
                    Ok(_) => info!("   No history items found"),
                    Err(e) => warn!("   Failed to read history: {}", e),
                }
            }

            // Include Cookies if requested
            if cookies {
                info!("🍪 Reading cookies...");
                match engine.get_all_cookies() {
                    Ok(items) if !items.is_empty() => {
                        info!("   {} cookies found", items.len());
                        let cookies_folder = browsers::Bookmark {
                            id: "cookies".to_string(),
                            title: "Cookies".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .enumerate()
                                .map(|(i, item)| browsers::Bookmark {
                                    id: format!("cookie-{i}"),
                                    title: format!("{} ({})", item.name, item.host),
                                    url: Some(format!("http://{}/{}", item.host, item.path)), // Fake URL for visualization
                                    folder: false,
                                    children: vec![],
                                    date_added: item.expiry,
                                    date_modified: None,
                                })
                                .collect(),
                            date_added: Some(chrono::Utc::now().timestamp_millis()),
                            date_modified: None,
                        };
                        extra_bookmarks.push(cookies_folder);
                    }
                    Ok(_) => info!("   No cookies found"),
                    Err(e) => warn!("   Failed to read cookies: {}", e),
                }
            }

            let export_config = sync::ExportConfig {
                merge,
                deduplicate,
                clean_empty: clean,
                verbose,
                folder_filter: folder.clone(),
                flat,
                wrap_folder: wrap.clone(),
                no_wrap,
                all_profiles,
            };

            // Show folder filter info
            if let Some(ref folder_name) = folder {
                info!("📁 Folder filter: \"{}\"", folder_name);
                info!("   Only bookmarks from folders matching this name will be exported");
            }

            // Show flat export info
            if flat {
                info!("📦 Flat export: browser root folders will be removed");
            }

            // Show wrap folder info
            if !no_wrap {
                let wrap_name = wrap.as_deref().unwrap_or("📁镜像文件夹");
                info!(
                    "📦 Wrap folder: all bookmarks will be inside \"{}\"",
                    wrap_name
                );
            }

            // Show update info
            if let Some(ref update_file) = update {
                info!("📝 Incremental update: merging with {}", update_file);
            }

            // Handle incremental update mode
            let count = if let Some(ref update_file) = update {
                // Read existing bookmarks from target file
                let expanded_update = expand_path(update_file);
                let mut existing_bookmarks =
                    match sync::import_bookmarks_from_html(&expanded_update) {
                        Ok(b) => b,
                        Err(e) => {
                            warn!(
                                "⚠️  Could not read existing file ({}), creating new file",
                                e
                            );
                            Vec::new()
                        }
                    };
                let existing_count = existing_bookmarks.iter().map(count_tree).sum::<usize>();

                // Get new bookmarks from browsers
                let new_bookmarks = engine
                    .collect_bookmarks_for_export(Some(&browsers), &export_config, extra_bookmarks)
                    .await?;

                // Merge new into existing
                let stats =
                    sync::merge_bookmarks_incremental(&mut existing_bookmarks, &new_bookmarks);
                info!(
                    "📊 Incremental update: {} new added, {} duplicates skipped",
                    stats.new_added, stats.skipped_duplicates
                );

                // Export merged result
                sync::export_bookmarks_to_html(&existing_bookmarks, &expand_path(&output))?;
                existing_count + stats.new_added
            } else {
                engine
                    .export_to_html_with_report(
                        Some(&browsers),
                        &output,
                        None,
                        &export_config,
                        extra_bookmarks,
                        external_source_reports,
                    )
                    .await?
            };

            info!("");
            info!("✅ Exported {} bookmarks to {}", count, output);

            if let Some(ref copy_path) = validate_copy {
                let expanded_copy = expand_path(copy_path);
                let expanded_output = expand_path(&output);
                info!(
                    "📋 Copying exported file to {} for validation...",
                    expanded_copy
                );
                if let Err(e) = std::fs::copy(&expanded_output, &expanded_copy) {
                    error!("❌ Failed to copy file for validation: {}", e);
                } else {
                    info!("✅ Copied to {}. Running validity check...", expanded_copy);
                    use std::collections::HashSet;
                    use url_checker::{collect_urls_from_bookmarks, CheckerConfig, UrlChecker};
                    if let Ok(bookmarks) = sync::import_bookmarks_from_html(&expanded_copy) {
                        let all_urls: HashSet<String> = collect_urls_from_bookmarks(&bookmarks)
                            .into_iter()
                            .collect();
                        let safe_conc = validate_concurrency.min(10);
                        let cfg = CheckerConfig {
                            proxy_url: None,
                            timeout_secs: 10,
                            concurrency: safe_conc,
                            retry_count: 1,
                        };
                        if let Ok(checker) = UrlChecker::new(cfg) {
                            use indicatif::{ProgressBar, ProgressStyle};
                            let mut url_vec: Vec<String> = all_urls.into_iter().collect();
                            if validate_limit > 0 && url_vec.len() > validate_limit {
                                info!(
                                    "📊 限制有效性检查数量: {} / {} URLs",
                                    validate_limit,
                                    url_vec.len()
                                );
                                url_vec.truncate(validate_limit);
                            }
                            let pb = ProgressBar::new(url_vec.len() as u64);
                            pb.set_style(
                                ProgressStyle::default_bar()
                                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                                    .unwrap_or_else(|_| ProgressStyle::default_bar())
                                    .progress_chars("#>-"),
                            );
                            let pb2 = pb.clone();
                            let results = if validate_content {
                                info!("🎯 启用【内容有效性检测 (Content Validity Mode)】: 针对主域名有效的前提下，深度排查具体页面内容是否返回 HTTP 404 Not Found / 410 Gone...");
                                checker
                                    .check_batch_content(url_vec, move |current, total, _url| {
                                        pb2.set_length(total as u64);
                                        pb2.set_position(current as u64);
                                    })
                                    .await
                            } else {
                                info!("🌐 启用【网站主域名去重有效性检测 (Domain Validity Mode)】: 结合大站白名单豁免，探测主域可达性...");
                                let pb3 = pb.clone();
                                checker
                                    .check_batch(url_vec, move |current, total, _url| {
                                        pb3.set_length(total as u64);
                                        pb3.set_position(current as u64);
                                    })
                                    .await
                            };
                            pb.finish_with_message("检查完成");
                            let total_checked = results.len();
                            let review_candidate_urls: HashSet<String> = results
                                .iter()
                                .filter(|r| r.status.should_auto_delete())
                                .map(|r| r.url.clone())
                                .collect();
                            let exempt_count =
                                results.iter().filter(|r| r.url.contains(".onion")).count();
                            let valid_count = results
                                .iter()
                                .filter(|r| {
                                    matches!(
                                        r.status,
                                        url_checker::ValidationStatus::Valid
                                            | url_checker::ValidationStatus::Redirected
                                    )
                                })
                                .count();
                            let doubtful_count = total_checked
                                .saturating_sub(review_candidate_urls.len() + valid_count);

                            let review_candidate_items: Vec<serde_json::Value> = results
                                .iter()
                                .filter(|r| r.status.should_auto_delete())
                                .map(|r| {
                                    serde_json::json!({
                                        "url": r.url,
                                        "status": format!("{:?}", r.status),
                                        "reason": r.error_message.clone().unwrap_or_else(|| "单轮检查命中失效候选 (DNS无法解析 / 服务器主动拒绝连接 / 404 Not Found / 410 Gone)".to_string()),
                                        "action": "候选待复核，校验副本保留 (Review candidate; validation copy retained)"
                                    })
                                })
                                .collect();

                            let doubtful_retained_items: Vec<serde_json::Value> = results
                                .iter()
                                .filter(|r| {
                                    !r.status.should_auto_delete()
                                        && !matches!(
                                            r.status,
                                            url_checker::ValidationStatus::Valid
                                                | url_checker::ValidationStatus::Redirected
                                        )
                                })
                                .map(|r| {
                                    serde_json::json!({
                                        "url": r.url,
                                        "status": format!("{:?}", r.status),
                                        "reason": r.error_message.clone().unwrap_or_else(|| "存疑保留 (本地网络与代理IO临时错误 / 网站暂时维护5xx / 403防护与429限流 / 暗网地址)".to_string()),
                                        "action": "存疑但保留 (Retained: local network IO error or temporary server maintenance)"
                                    })
                                })
                                .collect();

                            let validity_conclusions = serde_json::json!({
                                "validation_performed": true,
                                "validation_mode": if validate_content {
                                    "ContentValidity (内容有效性检测 - 针对主域名有效的前提下，排查具体完整页面路径返回 404/410 的失效候选)"
                                } else {
                                    "DomainValidity (网站主域名有效性检测 - 主层级域名去重 + 大站白名单豁免)"
                                },
                                "validated_copy_file_path": expanded_copy,
                                "summary": {
                                    "total_urls_checked": total_checked,
                                    "valid_or_redirected_retained_count": valid_count,
                                    "doubtful_retained_count": doubtful_count,
                                    "review_candidate_count": review_candidate_urls.len(),
                                    "onion_websites_exempt_count": exempt_count,
                                    "retained_checked_count": total_checked
                                },
                                "conclusions_and_rules": {
                                    "major_site_whitelist_rule": "根据‘常见且热门的互联网站点清单’规则（内置涵盖 YouTube, GitHub, Google, Baidu, Bing, Bilibili, Zhihu, Taobao, Twitter, Apple, Microsoft 等近 50 个核心平台，并支持通过外部 JSON 文件如 major_sites.json 进行热部署配置与扩展），系统自动豁免此类绝不可发生全站 404/倒闭的站点，直接赋予白名单跳过并保留，大幅缩减检测耗时且杜绝平台反爬防封锁。",
                                    "domain_level_validation_rule": "系统采用‘主层级域名去重校验’机制。例如 youtube.com 下所有视频仅检测主域是否存活，消减 90%+ 的重复网络请求并彻底解决 WAF/429 限流问题，凡主域名可正常访问的，下属所有书签均视为有效保留。",
                                    "content_validity_rule": "有关于内容有效性检测（--validate-content）：本功能为单独的深度内容有效性排查，检测前提是该网站主域名有效。系统对完整书签 URL 路径（例如具体的视频或贴文 URL）进行校验，当页面返回明确 HTTP 404 Not Found 或 410 Gone 时列为失效候选；反爬限流等情况继续存疑保留。",
                                    "review_candidate_rule": "单轮检查只能报告失效候选，不能作为破坏性删除依据。DNS无法解析、服务器主动拒绝连接或明确返回 404/410 的书签会列入候选清单，但校验副本保持原样，等待多轮历史确认与人工复核。",
                                    "retained_links_rule": "返回 HTTP 2xx (正常访问) 和 3xx (重定向) 的有效网址予以完全保留。",
                                    "doubtful_retained_rule": "基于死链判定五维度规则，对以下情况绝不误删并予以保留：【维度4】网络/网络代理原因导致的临时IO报错（连接超时/TLS握手错误/网络连接重置等 - LocalNetworkError/Timeout），判定为本地网络侧原因；【维度5】网站暂时维护但未来有望开放，以及开启反爬/WAF防护（HTTP 401, 403, 429, 502/503/504 ServiceUnavailable/ServerError 等）。以上全部列入‘存疑但保留’清单；同时针对 .onion 暗网地址亦免除检查予以保留。"
                                },
                                "review_candidate_items": review_candidate_items,
                                "doubtful_retained_items": doubtful_retained_items,
                                "validation_copy_preserved": true,
                                "status": "Completed (report only)"
                            });

                            info!("📊 Validity Check Summary:");
                            info!("   Total Checked: {}", total_checked);
                            info!("   Retained after review-only scan: {}", total_checked);
                            info!("   Onion Websites Exempted: {}", exempt_count);
                            info!(
                                "   Review candidates (copy unchanged): {}",
                                review_candidate_urls.len()
                            );

                            let report_path = {
                                let p = std::path::Path::new(&expanded_output);
                                let stem =
                                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
                                let dir = p.parent().unwrap_or(std::path::Path::new("."));
                                dir.join(format!("{stem}_report.json"))
                                    .to_string_lossy()
                                    .to_string()
                            };
                            if let Ok(content) = std::fs::read_to_string(&report_path) {
                                if let Ok(mut json_val) =
                                    serde_json::from_str::<serde_json::Value>(&content)
                                {
                                    json_val["validity_check_conclusions"] = validity_conclusions;
                                    if let Ok(updated_json) =
                                        serde_json::to_string_pretty(&json_val)
                                    {
                                        if let Err(e) = std::fs::write(&report_path, updated_json) {
                                            error!("❌ 更新报告 JSON 失败: {}", e);
                                        } else {
                                            info!(
                                                "📋 已将详细的有效性检测结论与清单更新至报告: {}",
                                                report_path
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if clear_after {
                if !unsafe_write {
                    error!("❌ Error: --clear-after requires --unsafe-write flag to confirm destructive operation");
                    return Ok(());
                }
                warn!("");
                print_sync_warning();
                engine.clear_bookmarks(&browsers, false).await?;
                info!("✅ Source bookmarks cleared");
            }
        }

        Commands::Analyze { browsers } => {
            info!("🔍 Analyzing bookmarks...");
            let engine = SyncEngine::new()?;
            engine.analyze_bookmarks(browsers.as_deref()).await?;
        }

        Commands::Organize {
            browsers,
            file,
            output,
            rules,
            deduplicate,
            clean,
            stats,
            dry_run,
            verbose,
            classifier,
            ollama_model,
            ollama_url,
            ollama_batch_size,
        } => {
            if let Some(input_files) = file {
                // 从本地文件整理（支持多个文件合并）
                let output_path = output.unwrap_or_else(|| {
                    let first_file = &input_files[0];
                    let path = std::path::Path::new(first_file);
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                    let ext = path.extension().unwrap_or_default().to_string_lossy();
                    if input_files.len() > 1 {
                        format!(
                            "merged_organized.{}",
                            if ext.is_empty() { "html" } else { &ext }
                        )
                    } else {
                        format!(
                            "{}_organized.{}",
                            stem,
                            if ext.is_empty() { "html" } else { &ext }
                        )
                    }
                });
                if input_files.len() > 1 {
                    info!("🧠 Organizing bookmarks from {} files:", input_files.len());
                    for f in &input_files {
                        info!("   📄 {}", f);
                    }
                } else {
                    info!("🧠 Organizing bookmarks from file: {}", input_files[0]);
                }
                info!("   📤 Output: {}", output_path);
                // 文件模式不需要初始化浏览器引擎
                sync::smart_organize_files(
                    &input_files.iter().map(String::as_str).collect::<Vec<_>>(),
                    &output_path,
                    rules.as_deref(),
                    &classifier,
                    &ollama_model,
                    &ollama_url,
                    ollama_batch_size,
                    deduplicate,
                    clean,
                    stats,
                    dry_run,
                    verbose,
                )
                .await?;
                if !dry_run {
                    info!("✅ Organized bookmarks saved to: {}", output_path);
                }
            } else {
                // 从浏览器整理
                if !dry_run {
                    print_sync_warning();
                }
                info!("🧠 Smart organizing bookmarks...");
                let mut engine = SyncEngine::new()?;
                engine
                    .smart_organize(
                        browsers.as_deref(),
                        rules.as_deref(),
                        &classifier,
                        &ollama_model,
                        &ollama_url,
                        ollama_batch_size,
                        false,
                        stats,
                        dry_run,
                        verbose,
                    )
                    .await?;
                info!("✅ Organization complete!");
            }
        }

        Commands::Validate { detailed } => {
            info!("🔍 Validating bookmarks...");
            let engine = SyncEngine::new()?;
            let report = engine.validate(detailed)?;
            println!("{report}");
        }

        Commands::History {
            browsers,
            days,
            dry_run,
            verbose,
        } => {
            info!("📜 Syncing browser history");
            info!("   Browsers: {}", browsers);
            info!("   Range: {} days", days);
            let mut engine = SyncEngine::new()?;
            engine.sync_history(Some(days), dry_run, verbose).await?;
            info!("✅ History sync complete!");
        }

        Commands::Rules => {
            SyncEngine::print_builtin_rules();
        }

        Commands::Health { command } => {
            command.execute().await.map_err(|e| anyhow::anyhow!(e))?;
        }

        Commands::Check {
            file,
            output,
            proxy,
            timeout,
            concurrency,
            delete,
            dry_run,
            verbose,
            browsers,
            limit,
            export_invalid,
            export_dir,
            keep_empty,
            slow,
            history_db,
            secondary,
        } => {
            use indicatif::{ProgressBar, ProgressStyle};
            use std::collections::HashSet;
            use url_checker::{
                collect_urls_from_bookmarks, extract_by_status_preserve_structure,
                remove_invalid_bookmarks_preserve_structure, CheckReport, CheckerConfig,
                RemoveConfig, UrlChecker, ValidationStatus,
            };

            // 从文件模式
            let from_file = file.is_some();

            if delete && history_db.is_none() {
                println!("Deletion requires --history-db and a multi-run ConfirmedDead verdict.");
                return Ok(());
            }
            if delete && !from_file {
                println!("Direct browser mutation is disabled: current adapters cannot identify every source profile safely.");
                println!("Use --file and --output to create an exact, backed-up cleaned copy.");
                return Ok(());
            }
            if delete && output.is_none() {
                println!("Deletion from an HTML input requires a separate --output path.");
                return Ok(());
            }

            info!("🔍 检查收藏夹URL有效性");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            if from_file {
                if let Some(ref file_path) = file {
                    info!("输入文件: {file_path}");
                }
                if let Some(ref out) = output {
                    info!("输出文件: {out}");
                }
            }
            if let Some(ref p) = proxy {
                info!("代理: {}", p);
            } else {
                info!("代理: 未配置 (仅直连模式)");
            }
            info!("超时: {}秒", timeout);

            // 安全限制：并发数最大 10，防止系统过载
            let safe_concurrency = concurrency.min(10);
            if concurrency > 10 {
                warn!("⚠️  并发数已限制为 10（原请求: {}）", concurrency);
            }
            info!("并发: {}", safe_concurrency);

            // 安全警告：无限制检查
            if limit == 0 {
                warn!("⚠️  警告: 无限制检查模式！大量书签可能导致系统过载");
                warn!("   建议使用 --limit 100 限制检查数量");
            }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            // 创建检查器
            let config = CheckerConfig {
                proxy_url: proxy.clone(),
                timeout_secs: timeout,
                concurrency: safe_concurrency,
                retry_count: 1,
            };

            let checker = match UrlChecker::new(config) {
                Ok(c) => c,
                Err(e) => {
                    error!("❌ 创建检查器失败: {}", e);
                    return Ok(());
                }
            };

            // 读取收藏夹 - 支持从文件或浏览器读取
            let mut all_bookmarks: Vec<(browsers::BrowserType, Vec<browsers::Bookmark>)> =
                Vec::new();
            let mut all_urls = Vec::new();
            let mut file_bookmarks: Option<Vec<browsers::Bookmark>> = None;

            if let Some(ref input_file) = file {
                // 从文件读取
                let expanded = expand_path(input_file);
                info!("📖 从文件读取: {}", expanded);
                match sync::import_bookmarks_from_html(&expanded) {
                    Ok(bookmarks) => {
                        let urls = collect_urls_from_bookmarks(&bookmarks);
                        let bookmark_count: usize = bookmarks.iter().map(count_tree).sum();
                        info!("   {} 个书签, {} 个URL", bookmark_count, urls.len());
                        all_urls.extend(urls);
                        file_bookmarks = Some(bookmarks);
                    }
                    Err(e) => {
                        error!("❌ 读取文件失败: {}", e);
                        return Ok(());
                    }
                }
            } else {
                // 从浏览器读取
                let _engine = SyncEngine::new()?;
                let browser_list: Vec<String> = browsers
                    .split(',')
                    .map(|s| s.trim().to_lowercase().replace('-', " "))
                    .collect();

                let matches_browser = |name: &str, filter: &str| -> bool {
                    let name_lower = name.to_lowercase();
                    let name_normalized = name_lower.replace('-', " ");
                    let filter_lower = filter.to_lowercase();

                    if name_lower == filter_lower || name_normalized == filter_lower {
                        return true;
                    }
                    if filter_lower == "brave" && name_normalized.contains("nightly") {
                        return false;
                    }
                    name_lower.contains(&filter_lower) || name_normalized.contains(&filter_lower)
                };

                for adapter in browsers::get_all_adapters() {
                    let name = adapter.browser_type().name();
                    if browsers == "all" || browser_list.iter().any(|b| matches_browser(name, b)) {
                        match adapter.read_all_bookmarks() {
                            Ok(bookmarks) => {
                                let urls = collect_urls_from_bookmarks(&bookmarks);
                                info!(
                                    "📖 {} : {} 个收藏夹",
                                    adapter.browser_type().name(),
                                    urls.len()
                                );
                                all_urls.extend(urls);
                                all_bookmarks.push((adapter.browser_type(), bookmarks));
                            }
                            Err(e) => {
                                warn!("⚠️  {} 读取失败: {}", adapter.browser_type().name(), e);
                            }
                        }
                    }
                }

                // The legacy adapter list is intentionally small. In historical health mode,
                // supplement it with the comprehensive installation/profile harvester so Arc,
                // Edge, Chromium, Vivaldi, beta channels, and named profiles are not missed.
                if browsers == "all" && (history_db.is_some() || slow) {
                    let temp = tempfile::tempdir()?;
                    match sync::collector::BookmarkCollector::collect_all(temp.path()) {
                        Ok((tree, inventory, _)) => {
                            let (canonical, _, _, _, _) =
                                sync::dedupe::DedupeEngine::deduplicate_tree(&tree, &[]);
                            info!(
                                "📚 全面 Profile 采集: {} 个浏览器安装 / {} 个 Profile / {} 个唯一 URL",
                                inventory.total_installations,
                                inventory.total_profiles,
                                canonical.len()
                            );
                            for warning in inventory.warnings {
                                warn!("⚠️  {warning}");
                            }
                            all_urls.extend(canonical.into_iter().map(|item| item.exact_url));
                        }
                        Err(error) => warn!("⚠️  全面 Profile 采集失败，保留适配器结果: {error}"),
                    }
                }
            }

            if all_urls.is_empty() {
                info!("没有找到收藏夹");
                return Ok(());
            }

            // 去重URL
            let mut unique_urls: Vec<String> = all_urls
                .into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();

            // 应用限制
            if limit > 0 && unique_urls.len() > limit {
                info!(
                    "📊 共 {} 个唯一URL，限制检查前 {} 个",
                    unique_urls.len(),
                    limit
                );
                unique_urls.truncate(limit);
            } else {
                info!("\n📊 共 {} 个唯一URL待检查", unique_urls.len());
            }

            // ── 新引擎路径: 提供 --history-db 或 --slow 时走 HealthEngine ──────────
            if history_db.is_some() || slow {
                let targets: Vec<ProbeTarget> = unique_urls
                    .iter()
                    .filter_map(|url| ProbeTarget::from_url(url.as_str(), url.as_str()).ok())
                    .collect();

                let sec_list: Vec<String> = secondary
                    .as_deref()
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                let db_path = history_db
                    .as_ref()
                    .map(|s| std::path::PathBuf::from(expand_path(s)));

                let out_path = export_dir
                    .as_ref()
                    .map(|d| std::path::PathBuf::from(expand_path(d)));

                let engine = match HealthEngine::new(HealthEngineOptions {
                    proxy_url: proxy.clone(),
                    timeout_secs: timeout,
                    concurrency: safe_concurrency,
                    slow_mode: slow,
                    allow_private_ips: false,
                    history_db_path: db_path,
                    secondary_backends: sec_list,
                    output_dir: out_path,
                }) {
                    Ok(e) => e,
                    Err(e) => {
                        error!("❌ 健康引擎初始化失败: {}", e);
                        return Ok(());
                    }
                };

                let pb = ProgressBar::new(targets.len() as u64);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                        .unwrap_or_else(|_| ProgressStyle::default_bar())
                        .progress_chars("#>-"),
                );
                let pb2 = pb.clone();

                let report = engine
                    .run_probes(targets, move |cur, _total, _url| {
                        pb2.set_position(cur as u64);
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?;

                pb.finish_with_message("检查完成");

                println!("\n📊 健康检查结果 (多后端多历史分析)");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("  总计检查:      {}", report.total_checked);
                println!("  ✅ 有效:       {}", report.live_count);
                println!("  🔄 已恢复:     {}", report.recovered_count);
                println!("  🛡️  受保护:    {}", report.protected_count);
                println!("  ⚠️  临时失败:  {}", report.temp_failure_count);
                println!("  🔴 可能失效:   {}", report.probable_dead_count);
                println!("  ☠️  确认失效:  {}", report.confirmed_dead_count);
                println!("  ❓ 需人工审核: {}", report.needs_review_count);
                println!("  ⏱️  耗时:       {:.2}秒", report.duration_secs);
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

                if history_db.is_some() {
                    println!("\n💾 历史已写入 DB。后续运行:");
                    println!("   bsync health quarantine   -- 查看需审核的书签");
                    println!("   bsync health rebuild-summary  -- 重建汇总统计");
                }

                if delete {
                    ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true)
                        .map_err(anyhow::Error::msg)?;
                    let confirmed: HashSet<String> =
                        report.confirmed_dead_urls.iter().cloned().collect();
                    let mut cleaned = file_bookmarks.clone().unwrap_or_default();
                    let stats = remove_invalid_bookmarks_preserve_structure(
                        &mut cleaned,
                        &confirmed,
                        &RemoveConfig {
                            keep_empty_folders: keep_empty,
                        },
                    );

                    if dry_run {
                        println!(
                            "\n🧪 预览：将精确删除 {} 个历史确认失效书签；未写入文件。",
                            stats.bookmarks_removed
                        );
                    } else if let Some(out) = output.as_ref() {
                        let out = expand_path(out);
                        let temp = format!("{out}.tmp-{}", uuid::Uuid::new_v4());
                        sync::export_bookmarks_to_html(&cleaned, &temp)?;
                        if std::path::Path::new(&out).exists() {
                            let backup = format!(
                                "{out}.backup-{}",
                                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
                            );
                            std::fs::copy(&out, &backup)?;
                            println!("  🛟 原输出已备份: {backup}");
                        }
                        std::fs::rename(&temp, &out)?;
                        println!(
                            "\n✅ 已保存清理副本: {out}（精确删除 {} 项，存活和存疑项均保留）",
                            stats.bookmarks_removed
                        );
                    }
                }
                return Ok(());
            }

            // ── 旧引擎路径 (无 --history-db / --slow) ─────────────────────────────
            // 创建进度条
            let pb = ProgressBar::new(unique_urls.len() as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"));

            // 执行检查
            let start_time = std::time::Instant::now();
            let results = checker
                .check_batch(unique_urls, |current, _total, url| {
                    pb.set_position(current as u64);
                    if verbose {
                        pb.set_message(url.to_string());
                    }
                })
                .await;
            pb.finish_with_message("检查完成");

            let duration = start_time.elapsed().as_secs_f64();
            let report = CheckReport::from_results(&results, duration);

            // 显示结果
            println!("\n📊 检查结果");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  总计检查:   {}", report.total_checked);
            println!("  ✅ 有效:    {}", report.valid_count);
            println!("  ❌ 无效:    {}", report.invalid_count);
            println!("  ❓ 不确定:  {}", report.uncertain_count);
            println!("  ⏭️  跳过:    {}", report.skipped_count);
            println!("  ⏱️  耗时:    {:.2}秒", report.check_duration_secs);
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            // 显示无效URL详情
            if verbose && !report.invalid_urls.is_empty() {
                println!("\n❌ 无效URL列表:");
                for invalid in &report.invalid_urls {
                    println!("  • {}", invalid.url);
                    if let Some(ref pe) = invalid.proxy_error {
                        println!("    代理: {pe}");
                    }
                    if let Some(ref de) = invalid.direct_error {
                        println!("    直连: {de}");
                    }
                }
            }

            // 收集无效URL
            let invalid_urls: HashSet<String> = results
                .iter()
                .filter(|r| r.status == ValidationStatus::Invalid)
                .map(|r| r.url.clone())
                .collect();

            // 收集各状态的 URL
            let valid_urls: HashSet<String> = results
                .iter()
                .filter(|r| r.status == ValidationStatus::Valid)
                .map(|r| r.url.clone())
                .collect();
            let uncertain_urls: HashSet<String> = results
                .iter()
                .filter(|r| r.status == ValidationStatus::Uncertain)
                .map(|r| r.url.clone())
                .collect();
            let skipped_urls: HashSet<String> = results
                .iter()
                .filter(|r| r.status == ValidationStatus::Skipped)
                .map(|r| r.url.clone())
                .collect();

            // 文件模式: 导出保留全部书签的检查副本
            if from_file {
                if let Some(ref out_path) = output {
                    let out_expanded = expand_path(out_path);
                    println!("\n📤 导出保留全部书签的检查副本到: {out_expanded}");

                    if let Some(ref bookmarks) = file_bookmarks {
                        // 旧扫描只有单轮结果，不能据此从默认输出中隔离任何书签。
                        match sync::export_bookmarks_to_html(bookmarks, &out_expanded) {
                            Ok(()) => {
                                let count: usize = bookmarks.iter().map(count_tree).sum();
                                info!("✅ 保留并导出了 {} 个书签到 {}", count, out_expanded);
                            }
                            Err(e) => error!("❌ 导出失败: {}", e),
                        }
                    }
                } else {
                    warn!("⚠️  文件模式需要指定 --output 参数来保存检查副本");
                }
            }

            // 导出所有分类到目录
            if let Some(ref dir) = export_dir {
                let dir_path = expand_path(dir);
                if let Err(e) = std::fs::create_dir_all(&dir_path) {
                    tracing::error!(
                        "无法创建导出目录 '{}': {}，后续导出文件可能会失败",
                        dir_path,
                        e
                    );
                }
                println!("\n📤 导出检查结果到: {dir_path}");

                // 根据模式选择书签源
                let source_bookmarks: Vec<&Vec<browsers::Bookmark>> = if from_file {
                    if let Some(b) = file_bookmarks.as_ref() {
                        vec![b]
                    } else {
                        tracing::warn!("指定了 --from-file 模式，但书签文件加载为空");
                        Vec::new()
                    }
                } else {
                    all_bookmarks.iter().map(|(_, b)| b).collect()
                };

                // 导出有效书签 (保持文件夹结构)
                if !valid_urls.is_empty() {
                    let mut valid_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &valid_urls);
                        valid_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/valid.html");
                    let actual_count: usize = valid_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&valid_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ✅ valid.html: {} 个有效书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ valid.html 导出失败: {}", e),
                    }
                }

                // 导出无效书签 (保持文件夹结构)
                if !invalid_urls.is_empty() {
                    let mut invalid_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &invalid_urls);
                        invalid_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/invalid.html");
                    let actual_count: usize = invalid_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&invalid_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ❌ invalid.html: {} 个无效书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ invalid.html 导出失败: {}", e),
                    }
                }

                // 导出不确定书签 (保持文件夹结构)
                if !uncertain_urls.is_empty() {
                    let mut uncertain_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &uncertain_urls);
                        uncertain_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/uncertain.html");
                    let actual_count: usize = uncertain_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&uncertain_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ❓ uncertain.html: {} 个不确定书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ uncertain.html 导出失败: {}", e),
                    }
                }

                // 导出跳过书签 (保持文件夹结构)
                if !skipped_urls.is_empty() {
                    let mut skipped_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &skipped_urls);
                        skipped_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/skipped.html");
                    let actual_count: usize = skipped_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&skipped_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ⏭️  skipped.html: {} 个跳过书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ skipped.html 导出失败: {}", e),
                    }
                }

                println!("✅ 导出完成");
            }

            // 导出无效收藏夹到HTML文件 (旧参数兼容, 保持文件夹结构)
            if let Some(ref export_path) = export_invalid {
                if !invalid_urls.is_empty() {
                    let export_path = expand_path(export_path);
                    println!("\n📤 导出无效收藏夹到: {export_path} (保持文件夹结构)");

                    let mut invalid_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    if from_file {
                        if let Some(ref bookmarks) = file_bookmarks {
                            let extracted =
                                extract_by_status_preserve_structure(bookmarks, &invalid_urls);
                            invalid_bookmarks.extend(extracted);
                        }
                    } else {
                        for (_browser_type, bookmarks) in &all_bookmarks {
                            let extracted =
                                extract_by_status_preserve_structure(bookmarks, &invalid_urls);
                            invalid_bookmarks.extend(extracted);
                        }
                    }

                    match sync::export_bookmarks_to_html(&invalid_bookmarks, &export_path) {
                        Ok(()) => info!(
                            "✅ 导出了 {} 个无效收藏夹到 {} (保持文件夹结构)",
                            invalid_bookmarks.len(),
                            export_path
                        ),
                        Err(e) => error!("❌ 导出失败: {}", e),
                    }
                }
            }
        }

        Commands::Backup { output } => {
            info!("💾 Creating backup...");
            sync::create_master_backup(&output, true).await?;
            info!("✅ Backup complete: {}", output);
        }

        Commands::ExportData {
            browser,
            passwords,
            cookies,
            downloads,
            all,
            output,
            format,
        } => {
            let output_dir = expand_path(&output);
            std::fs::create_dir_all(&output_dir)?;

            info!("🔐 Exporting browser data");
            info!("   Browser: {}", browser);
            info!("   Output: {}", output_dir);
            info!("   Format: {}", format);

            let export_passwords = passwords || all;
            let export_cookies = cookies || all;
            let export_downloads = downloads || all;

            // Get browser database paths
            let home = std::env::var("HOME")
                .map_err(|e| anyhow::anyhow!("获取 HOME 环境变量失败: {e}"))?;
            let db_base = match browser.to_lowercase().as_str() {
                "chrome" | "google chrome" => {
                    format!("{home}/Library/Application Support/Google/Chrome/Default")
                }
                "edge" | "microsoft edge" => {
                    format!("{home}/Library/Application Support/Microsoft Edge/Default")
                }
                "brave" => format!(
                    "{home}/Library/Application Support/BraveSoftware/Brave-Browser/Default"
                ),
                "arc" => format!("{home}/Library/Application Support/Arc/User Data/Default"),
                _ => {
                    error!("❌ Unsupported browser: {}", browser);
                    return Ok(());
                }
            };

            // Copy databases to temp for safety
            let temp_dir = std::path::Path::new("/tmp/browser-sync-export");
            std::fs::create_dir_all(temp_dir)?;

            if export_passwords {
                info!("🔑 Exporting passwords...");
                let login_db = format!("{db_base}/Login Data");
                let temp_db = temp_dir.join("LoginData");

                if std::path::Path::new(&login_db).exists() {
                    std::fs::copy(&login_db, &temp_db)?;

                    match data_types::extract_chromium_passwords(&temp_db, &browser) {
                        Ok(passwords) => {
                            let output_file = std::path::Path::new(&output_dir)
                                .join(format!("passwords_{browser}.{format}"));

                            match format.as_str() {
                                "json" => {
                                    data_types::password::export_to_json(&passwords, &output_file)?;
                                }
                                _ => data_types::password::export_to_csv(&passwords, &output_file)?,
                            }

                            info!(
                                "   ✅ {} passwords exported to {}",
                                passwords.len(),
                                output_file.display()
                            );
                        }
                        Err(e) => warn!("   ⚠️ Failed to export passwords: {}", e),
                    }

                    if let Err(e) = std::fs::remove_file(&temp_db) {
                        tracing::debug!("清理临时数据库拷贝 '{}' 失败: {}", temp_db.display(), e);
                    }
                } else {
                    warn!("   ⚠️ Login Data not found");
                }
            }

            if export_cookies {
                info!("🍪 Exporting cookies...");
                let cookies_db = format!("{db_base}/Cookies");
                let temp_db = temp_dir.join("Cookies");

                if std::path::Path::new(&cookies_db).exists() {
                    std::fs::copy(&cookies_db, &temp_db)?;

                    match data_types::extract_chromium_cookies(&temp_db, &browser) {
                        Ok(cookies) => {
                            let output_file = std::path::Path::new(&output_dir).join(format!(
                                "cookies_{}.{}",
                                browser,
                                if format == "netscape" { "txt" } else { &format }
                            ));

                            match format.as_str() {
                                "netscape" => {
                                    data_types::cookie::export_to_netscape(&cookies, &output_file)?;
                                }
                                "json" => {
                                    data_types::cookie::export_to_json(&cookies, &output_file)?;
                                }
                                _ => {
                                    // Simple CSV for cookies
                                    use std::io::Write;
                                    let mut file = std::fs::File::create(&output_file)?;
                                    writeln!(
                                        file,
                                        "host,name,value,path,expires,secure,http_only"
                                    )?;
                                    for c in &cookies {
                                        writeln!(
                                            file,
                                            "\"{}\",\"{}\",\"{}\",\"{}\",{},{},{}",
                                            c.host,
                                            c.name,
                                            c.value.replace('"', "\"\""),
                                            c.path,
                                            c.expires,
                                            c.is_secure,
                                            c.is_http_only
                                        )?;
                                    }
                                }
                            }

                            info!(
                                "   ✅ {} cookies exported to {}",
                                cookies.len(),
                                output_file.display()
                            );
                        }
                        Err(e) => warn!("   ⚠️ Failed to export cookies: {}", e),
                    }

                    if let Err(e) = std::fs::remove_file(&temp_db) {
                        tracing::debug!("清理临时数据库拷贝 '{}' 失败: {}", temp_db.display(), e);
                    }
                } else {
                    warn!("   ⚠️ Cookies database not found");
                }
            }

            if export_downloads {
                info!("📥 Exporting download history...");
                let history_db = format!("{db_base}/History");
                let temp_db = temp_dir.join("History");

                if std::path::Path::new(&history_db).exists() {
                    std::fs::copy(&history_db, &temp_db)?;

                    match data_types::extract_chromium_downloads(&temp_db, &browser) {
                        Ok(downloads) => {
                            let output_file = std::path::Path::new(&output_dir)
                                .join(format!("downloads_{browser}.{format}"));

                            data_types::download::export_to_csv(&downloads, &output_file)?;

                            info!(
                                "   ✅ {} downloads exported to {}",
                                downloads.len(),
                                output_file.display()
                            );
                        }
                        Err(e) => warn!("   ⚠️ Failed to export downloads: {}", e),
                    }

                    if let Err(e) = std::fs::remove_file(&temp_db) {
                        tracing::debug!("清理临时数据库拷贝 '{}' 失败: {}", temp_db.display(), e);
                    }
                } else {
                    warn!("   ⚠️ History database not found");
                }
            }

            info!("");
            info!("✅ Export complete: {}", output_dir);
        }

        Commands::Pipeline {
            output_dir,
            browsers,
            include,
            proxy,
            limit,
            concurrency,
            timeout,
        } => {
            let expanded_out_dir = expand_path(&output_dir);
            std::fs::create_dir_all(&expanded_out_dir)?;

            info!("🚀 Running Bookmark Processing Pipeline");
            info!("   Output Directory: {}", expanded_out_dir);

            // Step 1: Export
            let export_file = format!("{expanded_out_dir}/01_exported.html");
            let mut extra_bookmarks = Vec::new();
            for inc_path in &include {
                let expanded_inc = expand_path(inc_path);
                match sync::import_bookmarks_from_source(&expanded_inc) {
                    Ok(imported) => {
                        info!(
                            "   📥 Included {} bookmarks from {}",
                            imported.len(),
                            expanded_inc
                        );
                        extra_bookmarks.extend(imported);
                    }
                    Err(e) => {
                        error!(
                            "❌ Pipeline: 无法从外部数据源 '{}' 导入书签: {}",
                            expanded_inc, e
                        );
                        return Err(
                            e.context(format!("Pipeline 从数据源 '{expanded_inc}' 导入书签失败"))
                        );
                    }
                }
            }

            let export_cfg = sync::ExportConfig {
                merge: true,
                deduplicate: true,
                clean_empty: true,
                verbose: true,
                folder_filter: None,
                flat: true,
                wrap_folder: Some("📁镜像文件夹".to_string()),
                no_wrap: false,
                all_profiles: true,
            };

            let engine = SyncEngine::new()?;
            let exported_count = engine
                .export_to_html_with_extra(
                    Some(&browsers),
                    &export_file,
                    &export_cfg,
                    extra_bookmarks,
                )
                .await?;
            info!("   ✅ Step 1: Exported {} bookmarks", exported_count);

            // Step 2: Organize
            let organized_file = format!("{expanded_out_dir}/02_organized.html");
            if let Err(e) = sync::smart_organize_files(
                &[&export_file],
                &organized_file,
                None,
                "auto",
                "qwen2.5:7b",
                "http://127.0.0.1:11434",
                20,
                true,
                true,
                true,
                false,
                true,
            )
            .await
            {
                tracing::error!(
                    "智能书签组织 (Step 2: smart_organize_files) 执行失败: {}",
                    e
                );
            }
            info!("   ✅ Step 2: Organized bookmarks");

            // Step 3: Check
            let results_dir = format!("{expanded_out_dir}/03_check_results");
            std::fs::create_dir_all(&results_dir)?;

            let checker = url_checker::UrlChecker::new(url_checker::CheckerConfig {
                proxy_url: proxy,
                timeout_secs: timeout,
                concurrency,
                retry_count: 1,
            })?;

            if let Ok(imported_org) = sync::import_bookmarks_from_html(&organized_file) {
                let urls = url_checker::collect_urls_from_bookmarks(&imported_org);
                let mut unique_urls: Vec<String> = urls
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                if limit > 0 && unique_urls.len() > limit {
                    unique_urls.truncate(limit);
                }

                let results = checker.check_batch(unique_urls, |_, _, _| {}).await;

                let valid_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Valid)
                    .map(|r| r.url.clone())
                    .collect();
                let invalid_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Invalid)
                    .map(|r| r.url.clone())
                    .collect();
                let uncertain_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Uncertain)
                    .map(|r| r.url.clone())
                    .collect();
                let skipped_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Skipped)
                    .map(|r| r.url.clone())
                    .collect();

                let valid_bms =
                    url_checker::extract_by_status_preserve_structure(&imported_org, &valid_urls);
                let invalid_bms =
                    url_checker::extract_by_status_preserve_structure(&imported_org, &invalid_urls);
                let uncertain_bms = url_checker::extract_by_status_preserve_structure(
                    &imported_org,
                    &uncertain_urls,
                );
                let skipped_bms =
                    url_checker::extract_by_status_preserve_structure(&imported_org, &skipped_urls);

                if let Err(e) =
                    sync::export_bookmarks_to_html(&valid_bms, &format!("{results_dir}/valid.html"))
                {
                    tracing::error!("导出有效书签结果 (valid.html) 失败: {}", e);
                }
                if let Err(e) = sync::export_bookmarks_to_html(
                    &invalid_bms,
                    &format!("{results_dir}/invalid.html"),
                ) {
                    tracing::error!("导出无效书签结果 (invalid.html) 失败: {}", e);
                }
                if let Err(e) = sync::export_bookmarks_to_html(
                    &uncertain_bms,
                    &format!("{results_dir}/uncertain.html"),
                ) {
                    tracing::error!("导出不确定状态书签结果 (uncertain.html) 失败: {}", e);
                }
                if let Err(e) = sync::export_bookmarks_to_html(
                    &skipped_bms,
                    &format!("{results_dir}/skipped.html"),
                ) {
                    tracing::error!("导出跳过校验书签结果 (skipped.html) 失败: {}", e);
                }
            }

            // Step 4: Merge check results
            let final_file = format!("{expanded_out_dir}/FINAL_VALID.html");
            sync::merge_check_result_files(&results_dir, &final_file)?;
            info!("   ✅ Step 4: Final results saved to {}", final_file);
        }

        Commands::MergeResults { dir, output } => {
            let expanded_dir = expand_path(&dir);
            let expanded_output = expand_path(&output);
            sync::merge_check_result_files(&expanded_dir, &expanded_output)?;
        }

        Commands::WaterfoxCleanup { output } => {
            let expanded_output = expand_path(&output);
            let engine = SyncEngine::new()?;
            let config = sync::ExportConfig {
                merge: true,
                deduplicate: true,
                clean_empty: true,
                verbose: true,
                folder_filter: None,
                flat: true,
                wrap_folder: Some("📁镜像文件夹".to_string()),
                no_wrap: false,
                all_profiles: true,
            };
            engine
                .export_to_html_with_extra(Some("waterfox"), &expanded_output, &config, vec![])
                .await?;
            info!("✅ Waterfox cleanup export complete: {}", expanded_output);
        }

        Commands::Gui { port } => {
            gui::start_gui(port).await?;
        }
    }

    Ok(())
}

fn count_tree(bookmark: &browsers::Bookmark) -> usize {
    let mut count = usize::from(bookmark.url.is_some());
    for child in &bookmark.children {
        count += count_tree(child);
    }
    count
}
