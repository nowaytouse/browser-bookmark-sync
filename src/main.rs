use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

mod browser_utils;
mod browsers;
mod chromium_sync;
mod cleanup;
mod cloud_reset;
mod crypto;
mod data_types;
mod db_safety;
mod enhanced_rules;
mod firefox_sync;
mod firefox_sync_api;
mod hackbrowserdata;
mod scheduler;
mod sync;
mod sync_flags;
mod url_checker;
mod validator;

use sync::SyncEngine;
use sync_flags::SyncFlags;

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

        /// Import from existing HTML file
        #[arg(long)]
        include: Option<String>,

        /// Clear source browsers after export (⚠️  DANGEROUS!)
        #[arg(long)]
        clear_after: bool,

        /// Enable unsafe database writes (required for clear_after)
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

        /// Proxy server URL (e.g., http://127.0.0.1:7890)
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

        /// Include extra HTML file to merge
        #[arg(short, long)]
        include: Option<String>,

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

            // Import from existing HTML if specified
            let mut extra_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
            if let Some(html_path) = &include {
                let expanded = expand_path(html_path);
                info!("📥 Importing: {}", expanded);
                match sync::import_bookmarks_from_html(&expanded) {
                    Ok(bookmarks) => {
                        let count: usize = bookmarks.iter().map(count_tree).sum();
                        info!("   {} bookmarks imported", count);
                        extra_bookmarks = bookmarks;
                    }
                    Err(e) => warn!("   Import failed: {}", e),
                }
            }

            // Include Safari reading list if requested
            if reading_list {
                info!("📖 Reading Safari reading list...");
                match engine.get_safari_reading_list() {
                    Ok(items) if !items.is_empty() => {
                        info!("   {} items found", items.len());
                        let reading_folder = crate::browsers::Bookmark {
                            id: "reading-list".to_string(),
                            title: "Reading List".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .map(|item| crate::browsers::Bookmark {
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
                        let history_folder = crate::browsers::Bookmark {
                            id: "history".to_string(),
                            title: "History".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .enumerate()
                                .map(|(i, item)| crate::browsers::Bookmark {
                                    id: format!("hist-{}", i),
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
                        let cookies_folder = crate::browsers::Bookmark {
                            id: "cookies".to_string(),
                            title: "Cookies".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .enumerate()
                                .map(|(i, item)| crate::browsers::Bookmark {
                                    id: format!("cookie-{}", i),
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
                    .export_to_html_with_extra(
                        Some(&browsers),
                        &output,
                        &export_config,
                        extra_bookmarks,
                    )
                    .await?
            };

            info!("");
            info!("✅ Exported {} bookmarks to {}", count, output);

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
                    &input_files.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
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
            println!("{}", report);
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

            info!("🔍 检查收藏夹URL有效性");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            if from_file {
                info!("输入文件: {}", file.as_ref().unwrap());
                if let Some(ref out) = output {
                    info!("输出文件: {}", out);
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
            let mut all_bookmarks: Vec<(
                crate::browsers::BrowserType,
                Vec<crate::browsers::Bookmark>,
            )> = Vec::new();
            let mut all_urls = Vec::new();
            let mut file_bookmarks: Option<Vec<crate::browsers::Bookmark>> = None;

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

                for adapter in crate::browsers::get_all_adapters() {
                    let name = adapter.browser_type().name();
                    if browsers == "all" || browser_list.iter().any(|b| matches_browser(name, b)) {
                        match adapter.read_bookmarks() {
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

            // 创建进度条
            let pb = ProgressBar::new(unique_urls.len() as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("#>-"));

            // 执行检查
            let start_time = std::time::Instant::now();
            let results = checker
                .check_batch(unique_urls, |current, _total, url| {
                    pb.set_position(current as u64);
                    if verbose {
                        pb.set_message(format!("{}", url));
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
                        println!("    代理: {}", pe);
                    }
                    if let Some(ref de) = invalid.direct_error {
                        println!("    直连: {}", de);
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

            // 文件模式: 导出有效书签到输出文件
            if from_file {
                if let Some(ref out_path) = output {
                    let out_expanded = expand_path(out_path);
                    println!("\n📤 导出有效书签到: {}", out_expanded);

                    if let Some(ref bookmarks) = file_bookmarks {
                        // 移除无效和不确定的书签，保留有效和跳过的
                        let keep_urls: HashSet<String> =
                            valid_urls.union(&skipped_urls).cloned().collect();
                        let valid_bookmarks =
                            extract_by_status_preserve_structure(bookmarks, &keep_urls);

                        match sync::export_bookmarks_to_html(&valid_bookmarks, &out_expanded) {
                            Ok(_) => {
                                let count: usize = valid_bookmarks.iter().map(count_tree).sum();
                                info!("✅ 导出了 {} 个有效书签到 {}", count, out_expanded);
                            }
                            Err(e) => error!("❌ 导出失败: {}", e),
                        }
                    }
                } else {
                    warn!("⚠️  文件模式需要指定 --output 参数来保存有效书签");
                }
            }

            // 导出所有分类到目录
            if let Some(ref dir) = export_dir {
                let dir_path = expand_path(dir);
                std::fs::create_dir_all(&dir_path).ok();
                println!("\n📤 导出检查结果到: {}", dir_path);

                // 根据模式选择书签源
                let source_bookmarks: Vec<&Vec<crate::browsers::Bookmark>> = if from_file {
                    file_bookmarks.as_ref().map(|b| vec![b]).unwrap_or_default()
                } else {
                    all_bookmarks.iter().map(|(_, b)| b).collect()
                };

                // 导出有效书签 (保持文件夹结构)
                if !valid_urls.is_empty() {
                    let mut valid_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &valid_urls);
                        valid_bookmarks.extend(extracted);
                    }
                    let path = format!("{}/valid.html", dir_path);
                    let actual_count: usize = valid_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&valid_bookmarks, &path) {
                        Ok(_) => info!(
                            "  ✅ valid.html: {} 个有效书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ valid.html 导出失败: {}", e),
                    }
                }

                // 导出无效书签 (保持文件夹结构)
                if !invalid_urls.is_empty() {
                    let mut invalid_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &invalid_urls);
                        invalid_bookmarks.extend(extracted);
                    }
                    let path = format!("{}/invalid.html", dir_path);
                    let actual_count: usize = invalid_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&invalid_bookmarks, &path) {
                        Ok(_) => info!(
                            "  ❌ invalid.html: {} 个无效书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ invalid.html 导出失败: {}", e),
                    }
                }

                // 导出不确定书签 (保持文件夹结构)
                if !uncertain_urls.is_empty() {
                    let mut uncertain_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &uncertain_urls);
                        uncertain_bookmarks.extend(extracted);
                    }
                    let path = format!("{}/uncertain.html", dir_path);
                    let actual_count: usize = uncertain_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&uncertain_bookmarks, &path) {
                        Ok(_) => info!(
                            "  ❓ uncertain.html: {} 个不确定书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ uncertain.html 导出失败: {}", e),
                    }
                }

                // 导出跳过书签 (保持文件夹结构)
                if !skipped_urls.is_empty() {
                    let mut skipped_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &skipped_urls);
                        skipped_bookmarks.extend(extracted);
                    }
                    let path = format!("{}/skipped.html", dir_path);
                    let actual_count: usize = skipped_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&skipped_bookmarks, &path) {
                        Ok(_) => info!(
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
                    println!("\n📤 导出无效收藏夹到: {} (保持文件夹结构)", export_path);

                    let mut invalid_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
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
                        Ok(_) => info!(
                            "✅ 导出了 {} 个无效收藏夹到 {} (保持文件夹结构)",
                            invalid_bookmarks.len(),
                            export_path
                        ),
                        Err(e) => error!("❌ 导出失败: {}", e),
                    }
                }
            }

            // 处理删除
            if delete && report.invalid_count > 0 {
                if dry_run {
                    println!("\n🏃 Dry-run模式 - 以下URL将被删除:");
                    for url in &invalid_urls {
                        println!("  • {}", url);
                    }
                    println!("\n共 {} 个URL将被删除 (实际未删除)", invalid_urls.len());
                    if keep_empty {
                        println!("📁 空文件夹将被保留 (--keep-empty)");
                    } else {
                        println!("📁 空文件夹将被删除 (默认行为)");
                    }
                } else {
                    println!("\n🗑️  正在删除无效收藏夹 (保持文件夹结构)...");

                    let remove_config = RemoveConfig {
                        keep_empty_folders: keep_empty,
                    };

                    for (browser_type, mut bookmarks) in all_bookmarks {
                        // 备份
                        for adapter in crate::browsers::get_all_adapters() {
                            if adapter.browser_type() == browser_type {
                                match adapter.backup_bookmarks() {
                                    Ok(path) => {
                                        info!("💾 {} 备份: {:?}", browser_type.name(), path)
                                    }
                                    Err(e) => warn!("⚠️  {} 备份失败: {}", browser_type.name(), e),
                                }

                                let stats = remove_invalid_bookmarks_preserve_structure(
                                    &mut bookmarks,
                                    &invalid_urls,
                                    &remove_config,
                                );

                                if stats.bookmarks_removed > 0 || stats.empty_folders_removed > 0 {
                                    match adapter.write_bookmarks(&bookmarks) {
                                        Ok(_) => {
                                            info!(
                                                "✅ {} 删除了 {} 个无效书签",
                                                browser_type.name(),
                                                stats.bookmarks_removed
                                            );
                                            if stats.empty_folders_removed > 0 {
                                                info!(
                                                    "   清理了 {} 个空文件夹",
                                                    stats.empty_folders_removed
                                                );
                                            }
                                            info!("   保留了 {} 个文件夹", stats.folders_preserved);
                                        }
                                        Err(e) => {
                                            error!("❌ {} 写入失败: {}", browser_type.name(), e)
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }

                    println!("\n✅ 删除完成 (文件夹结构已保持)");
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
            let home = std::env::var("HOME").unwrap_or_default();
            let db_base = match browser.to_lowercase().as_str() {
                "chrome" | "google chrome" => {
                    format!("{}/Library/Application Support/Google/Chrome/Default", home)
                }
                "edge" | "microsoft edge" => format!(
                    "{}/Library/Application Support/Microsoft Edge/Default",
                    home
                ),
                "brave" => format!(
                    "{}/Library/Application Support/BraveSoftware/Brave-Browser/Default",
                    home
                ),
                "arc" => format!("{}/Library/Application Support/Arc/User Data/Default", home),
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
                let login_db = format!("{}/Login Data", db_base);
                let temp_db = temp_dir.join("LoginData");

                if std::path::Path::new(&login_db).exists() {
                    std::fs::copy(&login_db, &temp_db)?;

                    match data_types::extract_chromium_passwords(&temp_db, &browser) {
                        Ok(passwords) => {
                            let output_file = std::path::Path::new(&output_dir)
                                .join(format!("passwords_{}.{}", browser, format));

                            match format.as_str() {
                                "json" => {
                                    data_types::password::export_to_json(&passwords, &output_file)?
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

                    let _ = std::fs::remove_file(&temp_db);
                } else {
                    warn!("   ⚠️ Login Data not found");
                }
            }

            if export_cookies {
                info!("🍪 Exporting cookies...");
                let cookies_db = format!("{}/Cookies", db_base);
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
                                    data_types::cookie::export_to_netscape(&cookies, &output_file)?
                                }
                                "json" => {
                                    data_types::cookie::export_to_json(&cookies, &output_file)?
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

                    let _ = std::fs::remove_file(&temp_db);
                } else {
                    warn!("   ⚠️ Cookies database not found");
                }
            }

            if export_downloads {
                info!("📥 Exporting download history...");
                let history_db = format!("{}/History", db_base);
                let temp_db = temp_dir.join("History");

                if std::path::Path::new(&history_db).exists() {
                    std::fs::copy(&history_db, &temp_db)?;

                    match data_types::extract_chromium_downloads(&temp_db, &browser) {
                        Ok(downloads) => {
                            let output_file = std::path::Path::new(&output_dir)
                                .join(format!("downloads_{}.{}", browser, format));

                            data_types::download::export_to_csv(&downloads, &output_file)?;

                            info!(
                                "   ✅ {} downloads exported to {}",
                                downloads.len(),
                                output_file.display()
                            );
                        }
                        Err(e) => warn!("   ⚠️ Failed to export downloads: {}", e),
                    }

                    let _ = std::fs::remove_file(&temp_db);
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
            let export_file = format!("{}/01_exported.html", expanded_out_dir);
            let mut extra_bookmarks = Vec::new();
            if let Some(ref inc_path) = include {
                let expanded_inc = expand_path(inc_path);
                if let Ok(imported) = sync::import_bookmarks_from_html(&expanded_inc) {
                    info!("   📥 Included {} bookmarks from {}", imported.len(), expanded_inc);
                    extra_bookmarks = imported;
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
            let organized_file = format!("{}/02_organized.html", expanded_out_dir);
            let _ = sync::smart_organize_files(
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
            );
            info!("   ✅ Step 2: Organized bookmarks");

            // Step 3: Check
            let results_dir = format!("{}/03_check_results", expanded_out_dir);
            std::fs::create_dir_all(&results_dir)?;

            let checker = crate::url_checker::UrlChecker::new(crate::url_checker::CheckerConfig {
                proxy_url: proxy,
                timeout_secs: timeout,
                concurrency,
                retry_count: 1,
            })?;

            if let Ok(imported_org) = sync::import_bookmarks_from_html(&organized_file) {
                let urls = url_checker::collect_urls_from_bookmarks(&imported_org);
                let mut unique_urls: Vec<String> = urls.into_iter().collect::<std::collections::HashSet<_>>().into_iter().collect();
                if limit > 0 && unique_urls.len() > limit {
                    unique_urls.truncate(limit);
                }

                let results = checker.check_batch(unique_urls, |_, _, _| {}).await;

                let valid_urls: std::collections::HashSet<String> = results.iter().filter(|r| r.status == url_checker::ValidationStatus::Valid).map(|r| r.url.clone()).collect();
                let invalid_urls: std::collections::HashSet<String> = results.iter().filter(|r| r.status == url_checker::ValidationStatus::Invalid).map(|r| r.url.clone()).collect();
                let uncertain_urls: std::collections::HashSet<String> = results.iter().filter(|r| r.status == url_checker::ValidationStatus::Uncertain).map(|r| r.url.clone()).collect();
                let skipped_urls: std::collections::HashSet<String> = results.iter().filter(|r| r.status == url_checker::ValidationStatus::Skipped).map(|r| r.url.clone()).collect();

                let valid_bms = url_checker::extract_by_status_preserve_structure(&imported_org, &valid_urls);
                let invalid_bms = url_checker::extract_by_status_preserve_structure(&imported_org, &invalid_urls);
                let uncertain_bms = url_checker::extract_by_status_preserve_structure(&imported_org, &uncertain_urls);
                let skipped_bms = url_checker::extract_by_status_preserve_structure(&imported_org, &skipped_urls);

                let _ = sync::export_bookmarks_to_html(&valid_bms, &format!("{}/valid.html", results_dir));
                let _ = sync::export_bookmarks_to_html(&invalid_bms, &format!("{}/invalid.html", results_dir));
                let _ = sync::export_bookmarks_to_html(&uncertain_bms, &format!("{}/uncertain.html", results_dir));
                let _ = sync::export_bookmarks_to_html(&skipped_bms, &format!("{}/skipped.html", results_dir));
            }

            // Step 4: Merge check results
            let final_file = format!("{}/FINAL_VALID.html", expanded_out_dir);
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
            engine.export_to_html_with_extra(Some("waterfox"), &expanded_output, &config, vec![]).await?;
            info!("✅ Waterfox cleanup export complete: {}", expanded_output);
        }
    }

    Ok(())
}

fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        path.replacen("~", &std::env::var("HOME").unwrap_or_default(), 1)
    } else {
        path.to_string()
    }
}

fn count_tree(bookmark: &crate::browsers::Bookmark) -> usize {
    let mut count = if bookmark.url.is_some() { 1 } else { 0 };
    for child in &bookmark.children {
        count += count_tree(child);
    }
    count
}
