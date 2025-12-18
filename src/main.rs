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
        #[arg(short, long, default_value = "~/Desktop/bookmarks.html")]
        output: String,

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

        /// Remove duplicate bookmarks/URLs
        #[arg(short, long)]
        deduplicate: bool,

        /// Merge into flat structure (no browser folders)
        #[arg(short, long)]
        merge: bool,

        /// Remove empty folders
        #[arg(long)]
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
        /// Target browsers
        #[arg(short, long)]
        browsers: Option<String>,

        /// Custom rules file (JSON)
        #[arg(short, long)]
        rules: Option<String>,

        /// Show statistics
        #[arg(short, long)]
        stats: bool,

        /// Preview only, no changes
        #[arg(long)]
        dry_run: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
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
        /// Proxy server URL (e.g., http://127.0.0.1:7890)
        #[arg(short, long)]
        proxy: Option<String>,

        /// Request timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,

        /// Number of concurrent requests
        #[arg(short, long, default_value = "10")]
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

        /// Target browsers (comma-separated, or 'all')
        #[arg(short, long, default_value = "all")]
        browsers: String,

        /// Limit number of URLs to check (0 = no limit)
        #[arg(short, long, default_value = "0")]
        limit: usize,
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
            };

            // Show folder filter info
            if let Some(ref folder_name) = folder {
                info!("📁 Folder filter: \"{}\"", folder_name);
                info!("   Only bookmarks from folders matching this name will be exported");
            }

            let count = engine
                .export_to_html_with_extra(
                    Some(&browsers),
                    &output,
                    &export_config,
                    extra_bookmarks,
                )
                .await?;

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
            rules,
            stats,
            dry_run,
            verbose,
        } => {
            if !dry_run {
                print_sync_warning();
            }
            info!("🧠 Smart organizing bookmarks...");
            let mut engine = SyncEngine::new()?;
            engine
                .smart_organize(
                    browsers.as_deref(),
                    rules.as_deref(),
                    false,
                    stats,
                    dry_run,
                    verbose,
                )
                .await?;
            info!("✅ Organization complete!");
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
            proxy,
            timeout,
            concurrency,
            delete,
            dry_run,
            verbose,
            browsers,
            limit,
        } => {
            use url_checker::{
                CheckerConfig, UrlChecker, CheckReport, ValidationStatus,
                collect_urls_from_bookmarks, remove_invalid_bookmarks,
            };
            use std::collections::HashSet;
            use indicatif::{ProgressBar, ProgressStyle};

            info!("🔍 检查收藏夹URL有效性");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            if let Some(ref p) = proxy {
                info!("代理: {}", p);
            } else {
                info!("代理: 未配置 (仅直连模式)");
            }
            info!("超时: {}秒", timeout);
            info!("并发: {}", concurrency);
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            // 创建检查器
            let config = CheckerConfig {
                proxy_url: proxy.clone(),
                timeout_secs: timeout,
                concurrency,
                retry_count: 1,
            };
            
            let checker = match UrlChecker::new(config) {
                Ok(c) => c,
                Err(e) => {
                    error!("❌ 创建检查器失败: {}", e);
                    return Ok(());
                }
            };

            // 读取收藏夹
            let _engine = SyncEngine::new()?; // 用于验证浏览器检测
            let browser_list: Vec<String> = browsers.split(',')
                .map(|s| s.trim().to_lowercase().replace('-', " ")) // 支持 brave-nightly 格式
                .collect();
            
            let mut all_bookmarks = Vec::new();
            let mut all_urls = Vec::new();
            
            // 精确匹配浏览器名称的辅助函数
            let matches_browser = |name: &str, filter: &str| -> bool {
                let name_lower = name.to_lowercase();
                let name_normalized = name_lower.replace('-', " ");
                let filter_lower = filter.to_lowercase();
                
                // 精确匹配或完整词匹配
                if name_lower == filter_lower || name_normalized == filter_lower {
                    return true;
                }
                // "brave" 不应该匹配 "brave nightly"，但 "nightly" 可以匹配 "brave nightly"
                if filter_lower == "brave" && name_normalized.contains("nightly") {
                    return false;
                }
                // 部分匹配（用于 "nightly" 匹配 "brave nightly"）
                name_lower.contains(&filter_lower) || name_normalized.contains(&filter_lower)
            };
            
            for adapter in crate::browsers::get_all_adapters() {
                let name = adapter.browser_type().name();
                if browsers == "all" || browser_list.iter().any(|b| matches_browser(name, b)) {
                    match adapter.read_bookmarks() {
                        Ok(bookmarks) => {
                            let urls = collect_urls_from_bookmarks(&bookmarks);
                            info!("📖 {} : {} 个收藏夹", adapter.browser_type().name(), urls.len());
                            all_urls.extend(urls);
                            all_bookmarks.push((adapter.browser_type(), bookmarks));
                        }
                        Err(e) => {
                            warn!("⚠️  {} 读取失败: {}", adapter.browser_type().name(), e);
                        }
                    }
                }
            }

            if all_urls.is_empty() {
                info!("没有找到收藏夹");
                return Ok(());
            }

            // 去重URL
            let mut unique_urls: Vec<String> = all_urls.into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            
            // 应用限制
            if limit > 0 && unique_urls.len() > limit {
                info!("📊 共 {} 个唯一URL，限制检查前 {} 个", unique_urls.len(), limit);
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
            let results = checker.check_batch(unique_urls, |current, _total, url| {
                pb.set_position(current as u64);
                if verbose {
                    pb.set_message(format!("{}", url));
                }
            }).await;
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

            // 处理删除
            if delete && report.invalid_count > 0 {
                let invalid_urls: HashSet<String> = results.iter()
                    .filter(|r| r.status == ValidationStatus::Invalid)
                    .map(|r| r.url.clone())
                    .collect();

                if dry_run {
                    println!("\n🏃 Dry-run模式 - 以下URL将被删除:");
                    for url in &invalid_urls {
                        println!("  • {}", url);
                    }
                    println!("\n共 {} 个URL将被删除 (实际未删除)", invalid_urls.len());
                } else {
                    println!("\n🗑️  正在删除无效收藏夹...");
                    
                    for (browser_type, mut bookmarks) in all_bookmarks {
                        // 备份
                        for adapter in crate::browsers::get_all_adapters() {
                            if adapter.browser_type() == browser_type {
                                match adapter.backup_bookmarks() {
                                    Ok(path) => info!("💾 {} 备份: {:?}", browser_type.name(), path),
                                    Err(e) => warn!("⚠️  {} 备份失败: {}", browser_type.name(), e),
                                }
                                
                                let removed = remove_invalid_bookmarks(&mut bookmarks, &invalid_urls);
                                if removed > 0 {
                                    match adapter.write_bookmarks(&bookmarks) {
                                        Ok(_) => info!("✅ {} 删除了 {} 个无效收藏夹", browser_type.name(), removed),
                                        Err(e) => error!("❌ {} 写入失败: {}", browser_type.name(), e),
                                    }
                                }
                                break;
                            }
                        }
                    }
                    
                    println!("\n✅ 删除完成");
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
