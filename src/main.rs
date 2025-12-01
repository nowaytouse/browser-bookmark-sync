use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{info, warn};

mod browsers;
mod sync;
mod scheduler;
mod validator;
mod firefox_sync;
mod firefox_sync_api;
mod cloud_reset;
mod cleanup;
mod browser_utils;


use sync::SyncEngine;
use scheduler::SchedulerConfig;

#[derive(Parser)]
#[command(name = "browser-bookmark-sync")]
#[command(about = "Reliable cross-browser bookmark synchronization tool", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Full sync between hub browsers (bookmarks + history + cookies)
    #[command(alias = "s")]
    Sync {
        /// Hub browsers (comma-separated). Use "all" for all browsers
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// Sync mode: 
        /// - bidirectional-incremental: 双向增量同步 (检测变更,双向合并)
        /// - bidirectional-full: 双向全量同步 (读取所有,双向合并)
        /// - specified-incremental: 指定浏览器增量同步
        /// - specified-full: 指定浏览器全量同步
        #[arg(short = 'm', long, default_value = "bidirectional-incremental")]
        mode: String,
        
        /// Clear data from non-hub browsers (only for bidirectional modes)
        #[arg(long)]
        clear_others: bool,
        
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
        
        /// Firefox Sync strategy: ignore, warn, trigger, wait, or api
        #[arg(long, default_value = "api")]
        firefox_sync: String,
        
        /// Automatically close target browsers before syncing
        #[arg(long)]
        auto_close_browsers: bool,
    },
    
    /// Start the scheduler for automatic periodic syncing
    Schedule {
        /// Cron expression (default: "0 */30 * * * *" - every 30 minutes)
        #[arg(short, long, default_value = "0 */30 * * * *")]
        cron: String,
        
        /// Run as daemon
        #[arg(short, long)]
        daemon: bool,
    },
    
    /// Validate bookmark integrity across all browsers
    #[command(alias = "v", alias = "check")]
    Validate {
        /// Show detailed validation report
        #[arg(short, long)]
        detailed: bool,
    },
    
    /// List all detected browsers and their bookmark locations
    #[command(alias = "l", alias = "ls")]
    List,
    
    /// Import bookmarks from Safari HTML export
    ImportSafari {
        /// Path to Safari HTML export file
        #[arg(short, long)]
        file: String,
        
        /// Target browser to import into
        #[arg(short, long, default_value = "all")]
        target: String,
    },
    
    /// Synchronize browsing history across browsers (syncs ALL history)
    SyncHistory {
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Synchronize reading lists across browsers
    SyncReadingList {
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    
    /// Synchronize cookies across browsers
    SyncCookies {
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Sync cookies to hub browsers (collect all to Brave Nightly, then sync to Waterfox)
    SyncCookiesToHub {
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Set hub browsers and sync ALL data between them (bookmarks, history, cookies)
    SetHubs {
        /// Hub browsers (comma-separated, e.g., "waterfox,brave-nightly")
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// Skip history sync
        #[arg(long)]
        no_history: bool,
        
        /// Skip cookies sync
        #[arg(long)]
        no_cookies: bool,
        
        /// Clear data from non-hub browsers after migration
        #[arg(long)]
        clear_others: bool,
        
        /// Dry run - show what would be done without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Synchronize specific scenario folder across browsers
    SyncScenario {
        /// Scenario folder path (e.g., "Work/Projects" or "Personal/Finance")
        #[arg(short = 'p', long)]
        scenario_path: String,
        
        /// Target browsers (comma-separated)
        #[arg(short = 'b', long)]
        browsers: String,
        
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Clean up bookmarks (remove duplicates and/or empty folders)
    #[command(alias = "c", alias = "clean")]
    Cleanup {
        /// Target browsers (comma-separated, default: all browsers)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// Remove duplicate bookmarks
        #[arg(long)]
        remove_duplicates: bool,
        
        /// Remove empty bookmark folders
        #[arg(long)]
        remove_empty_folders: bool,
        
        /// Dry run - show what would be cleaned without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Organize homepage bookmarks into dedicated folder
    #[command(alias = "o", alias = "org")]
    Organize {
        /// Target browsers (comma-separated, default: all browsers)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// Dry run - show what would be organized without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Smart organize bookmarks using rule engine (auto-classify by URL patterns)
    #[command(alias = "so", alias = "smart")]
    SmartOrganize {
        /// Target browsers (comma-separated, default: all browsers)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// Path to custom rules file (JSON format)
        #[arg(short = 'r', long)]
        rules_file: Option<String>,
        
        /// Only organize uncategorized bookmarks (not in folders)
        #[arg(long)]
        uncategorized_only: bool,
        
        /// Show rule matching statistics
        #[arg(long)]
        show_stats: bool,
        
        /// Dry run - show what would be organized without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// List available classification rules
    ListRules,
    
    /// Reset Firefox Sync cloud and sync fresh data (solves cloud override issue)
    CloudReset {
        /// Skip confirmation prompts
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Migrate all data to Safari and clear other browsers
    MigrateToSafari {
        /// Skip confirmation prompts
        #[arg(short = 'y', long)]
        yes: bool,

        /// Dry run - show what would be migrated without making changes
        #[arg(short, long)]
        dry_run: bool,

        /// Keep data in source browsers (don't clear after migration)
        #[arg(long)]
        keep_source: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Analyze bookmarks for anomalies (bulk imports, history pollution, NSFW)
    #[command(alias = "a")]
    Analyze {
        /// Target browsers (comma-separated, default: all browsers)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
    },
    
    // DeepClean命令已移除 - 自动删除功能误删风险太高
    
    /// Restore bookmarks from backup
    RestoreBackup {
        /// Browser to restore (e.g., "waterfox")
        #[arg(short = 'b', long)]
        browser: String,
        
        /// Backup file path (optional, uses latest backup if not specified)
        #[arg(short = 'f', long)]
        file: Option<String>,
    },
    
    /// Create comprehensive master backup from all browser data
    MasterBackup {
        /// Output directory for master backup
        #[arg(short = 'o', long, default_value = "~/Library/Safari/MasterBackup")]
        output: String,
        
        /// Include full data (not just unique URLs)
        #[arg(long)]
        include_full: bool,
    },
    
    /// Export bookmarks to HTML file (RECOMMENDED - let users import manually)
    #[command(alias = "export", alias = "e")]
    ExportHtml {
        /// Output HTML file path
        #[arg(short = 'o', long, default_value = "~/Desktop/bookmarks_export.html")]
        output: String,
        
        /// Source browsers (comma-separated, default: all)
        #[arg(short = 'b', long, default_value = "all")]
        browsers: String,
        
        /// Merge all bookmarks into flat structure (no browser folders)
        #[arg(long)]
        merge: bool,
        
        /// Remove duplicate bookmarks
        #[arg(long, short = 'd')]
        deduplicate: bool,
        
        /// Also import from existing HTML backup files
        #[arg(long)]
        include_html: Option<String>,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Clear bookmarks from specified browsers (DEBUG ONLY - use with caution!)
    #[command(alias = "clear")]
    ClearBookmarks {
        /// Target browsers (comma-separated)
        #[arg(short = 'b', long)]
        browsers: String,
        
        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
        
        /// Dry run - show what would be cleared
        #[arg(short, long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Sync { browsers, mode, clear_others, dry_run, verbose, firefox_sync, auto_close_browsers } => {
            // 解析同步模式
            let (is_bidirectional, is_incremental) = match mode.to_lowercase().as_str() {
                "bidirectional-incremental" | "bi-inc" => {
                    info!("🔄 模式: 双向增量同步 (检测变更,双向合并)");
                    (true, true)
                }
                "bidirectional-full" | "bi-full" => {
                    info!("🔄 模式: 双向全量同步 (读取所有,双向合并)");
                    (true, false)
                }
                "specified-incremental" | "spec-inc" => {
                    info!("🔄 模式: 指定浏览器增量同步");
                    (false, true)
                }
                "specified-full" | "spec-full" => {
                    info!("🔄 模式: 指定浏览器全量同步");
                    (false, false)
                }
                // 兼容旧命令
                "incremental" | "inc" => {
                    info!("🔄 模式: 双向增量同步 (兼容模式)");
                    (true, true)
                }
                "full" => {
                    info!("🔄 模式: 双向全量同步 (兼容模式)");
                    (true, false)
                }
                _ => {
                    eprintln!("❌ Invalid sync mode: {}", mode);
                    eprintln!("Valid modes:");
                    eprintln!("  - bidirectional-incremental: 双向增量同步");
                    eprintln!("  - bidirectional-full: 双向全量同步");
                    eprintln!("  - specified-incremental: 指定浏览器增量同步");
                    eprintln!("  - specified-full: 指定浏览器全量同步");
                    std::process::exit(1);
                }
            };
            
            // 解析Firefox Sync策略
            let firefox_sync_strategy = match firefox_sync.to_lowercase().as_str() {
                "ignore" => firefox_sync::SyncStrategy::Ignore,
                "warn" => firefox_sync::SyncStrategy::WarnAndContinue,
                "trigger" => firefox_sync::SyncStrategy::TriggerSync,
                "wait" => firefox_sync::SyncStrategy::TriggerAndWait { timeout_secs: 60 },
                "api" => firefox_sync::SyncStrategy::UseAPI,
                _ => {
                    eprintln!("❌ Invalid firefox-sync strategy: {}. Use 'ignore', 'warn', 'trigger', 'wait', or 'api'", firefox_sync);
                    std::process::exit(1);
                }
            };
            
            info!("🎯 目标浏览器: {}", browsers);
            
            // Auto-close browsers if requested
            if auto_close_browsers && !dry_run {
                let browser_list = browser_utils::parse_browser_list(&browsers);
                browser_utils::close_browsers(&browser_list, false)?;
            }
            
            let mut engine = SyncEngine::new()?;
            
            if is_bidirectional {
                // 双向同步 (原有逻辑)
                if is_incremental {
                    // 双向增量: 使用增量sync
                    info!("🔄 执行双向增量同步...");
                    info!("  (增量检测功能开发中,当前使用全量逻辑)");
                    engine.set_hub_browsers_with_firefox_sync(
                        &browsers, 
                        true, true, true, 
                        clear_others, 
                        dry_run, verbose,
                        firefox_sync_strategy
                    ).await?;
                } else {
                    // 双向全量: 当前的Base & Merge逻辑
                    info!("🔄 执行双向全量同步 (Base & Merge)...");
                    engine.set_hub_browsers_with_firefox_sync(
                        &browsers, 
                        true, true, true, 
                        clear_others, 
                        dry_run, verbose,
                        firefox_sync_strategy
                    ).await?;
                }
            } else {
                // 指定浏览器同步
                if is_incremental {
                    // 指定增量
                    info!("🔄 执行指定浏览器增量同步...");
                    info!("  (增量检测功能开发中,当前使用全量逻辑)");
                    engine.set_hub_browsers_with_firefox_sync(
                        &browsers, 
                        true, true, true, 
                        false,  // 不清空其他
                        dry_run, verbose,
                        firefox_sync_strategy
                    ).await?;
                } else {
                    // 指定全量
                    info!("🔄 执行指定浏览器全量同步...");
                    engine.set_hub_browsers_with_firefox_sync(
                        &browsers, 
                        true, true, true, 
                        false,  // 不清空其他
                        dry_run, verbose,
                        firefox_sync_strategy
                    ).await?;
                }
            }
            
            info!("✅ Synchronization complete!");
        }
        
        Commands::Schedule { cron, daemon } => {
            info!("⏰ Starting scheduler with cron: {}", cron);
            let config = SchedulerConfig::new(cron, daemon);
            scheduler::start_scheduler(config).await?;
        }
        
        Commands::Validate { detailed } => {
            info!("🔍 Validating bookmarks...");
            let engine = SyncEngine::new()?;
            let report = engine.validate(detailed)?;
            println!("{}", report);
        }
        
        Commands::List => {
            info!("📋 Listing detected browsers...");
            let engine = SyncEngine::new()?;
            engine.list_browsers()?;
        }
        
        Commands::ImportSafari { file, target } => {
            info!("📥 Importing Safari bookmarks from: {}", file);
            let mut engine = SyncEngine::new()?;
            engine.import_safari_html(&file, &target).await?;
            info!("✅ Import complete!");
        }
        
        Commands::SyncHistory { dry_run, verbose } => {
            info!("📜 Starting history synchronization (ALL history)...");
            let mut engine = SyncEngine::new()?;
            engine.sync_history(None, dry_run, verbose).await?;
            info!("✅ History synchronization complete!");
        }
        
        Commands::SyncReadingList { dry_run, verbose } => {
            info!("📚 Starting reading list synchronization...");
            let mut engine = SyncEngine::new()?;
            engine.sync_reading_list(dry_run, verbose).await?;
            info!("✅ Reading list synchronization complete!");
        }
        
        Commands::SyncCookies { dry_run, verbose } => {
            info!("🍪 Starting cookies synchronization...");
            let mut engine = SyncEngine::new()?;
            engine.sync_cookies(dry_run, verbose).await?;
            info!("✅ Cookies synchronization complete!");
        }
        
        Commands::SyncCookiesToHub { dry_run, verbose } => {
            info!("🍪 Starting cookies sync to hub browsers...");
            let mut engine = SyncEngine::new()?;
            engine.sync_cookies_to_hub(dry_run, verbose).await?;
            info!("✅ Cookies hub synchronization complete!");
        }
        
        Commands::SetHubs { browsers, no_history, no_cookies, clear_others, dry_run, verbose } => {
            info!("🎯 Setting hub browsers: {}", browsers);
            let mut engine = SyncEngine::new()?;
            // Default: sync ALL data (history, reading list, cookies) unless explicitly disabled
            let sync_history = !no_history;
            let sync_reading_list = true; // Always sync reading list
            let sync_cookies = !no_cookies;
            engine.set_hub_browsers(&browsers, sync_history, sync_reading_list, sync_cookies, clear_others, dry_run, verbose).await?;
            info!("✅ Hub configuration complete!");
        }
        
        Commands::SyncScenario { scenario_path, browsers, dry_run, verbose } => {
            info!("📁 Starting scenario folder synchronization");
            info!("🎯 Scenario: {}", scenario_path);
            info!("🌐 Browsers: {}", browsers);
            let mut engine = SyncEngine::new()?;
            engine.sync_scenario_folders(&scenario_path, &browsers, dry_run, verbose).await?;
            info!("✅ Scenario synchronization complete!");
        }
        
        Commands::Cleanup { browsers, remove_duplicates, remove_empty_folders, dry_run, verbose } => {
            if !remove_duplicates && !remove_empty_folders {
                eprintln!("⚠️  Please specify at least one cleanup option:");
                eprintln!("   --remove-duplicates       Remove duplicate bookmarks");
                eprintln!("   --remove-empty-folders    Remove empty bookmark folders");
                std::process::exit(1);
            }
            
            info!("🧹 Starting bookmark cleanup");
            if remove_duplicates {
                info!("  🔄 Will remove duplicate bookmarks");
            }
            if remove_empty_folders {
                info!("  🗑️  Will remove empty folders");
            }
            
            let mut engine = SyncEngine::new()?;
            engine.cleanup_bookmarks(
                browsers.as_deref(),
                remove_duplicates,
                remove_empty_folders,
                dry_run,
                verbose
            ).await?;
            info!("✅ Cleanup complete!");
        }
        
        Commands::Organize { browsers, dry_run, verbose } => {
            info!("📋 Starting homepage organization");
            
            let mut engine = SyncEngine::new()?;
            engine.organize_homepages(
                browsers.as_deref(),
                dry_run,
                verbose
            ).await?;
            info!("✅ Organization complete!");
        }
        
        Commands::SmartOrganize { browsers, rules_file, uncategorized_only, show_stats, dry_run, verbose } => {
            info!("🧠 Starting smart bookmark organization");
            
            let mut engine = SyncEngine::new()?;
            engine.smart_organize(
                browsers.as_deref(),
                rules_file.as_deref(),
                uncategorized_only,
                show_stats,
                dry_run,
                verbose
            ).await?;
            info!("✅ Smart organization complete!");
        }
        
        Commands::ListRules => {
            SyncEngine::print_builtin_rules();
        }
        
        Commands::CloudReset { yes } => {
            info!("🔄 Firefox Sync Cloud Reset");
            info!("");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("⚠️  这将清空Firefox Sync云端的书签数据！");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("");
            info!("流程：");
            info!("  1. 清空Waterfox本地书签");
            info!("  2. 启动Waterfox，让Firefox Sync上传'空书签'到云端");
            info!("  3. 云端书签被清空");
            info!("  4. 写入我们清理后的书签");
            info!("  5. 再次启动Waterfox，让Firefox Sync上传新书签到云端");
            info!("");
            
            if !yes {
                print!("确认继续？(y/N): ");
                use std::io::{self, Write};
                io::stdout().flush().ok();
                
                let mut input = String::new();
                io::stdin().read_line(&mut input).ok();
                
                if !input.trim().eq_ignore_ascii_case("y") {
                    info!("❌ 已取消");
                    return Ok(());
                }
            }
            
            // Step 1: 确保Waterfox已关闭
            info!("");
            info!("📋 Step 1: 关闭Waterfox");
            let _ = std::process::Command::new("killall")
                .arg("waterfox-bin")
                .output();
            std::thread::sleep(std::time::Duration::from_secs(2));
            info!("✅ Waterfox已关闭");
            
            // Step 2: 清空本地书签
            info!("");
            info!("📋 Step 2: 清空本地书签");
            let waterfox_db = std::path::PathBuf::from(std::env::var("HOME")?)
                .join("Library/Application Support/Waterfox/Profiles/ll4fbmm0.default-release/places.sqlite");
            
            // 先备份
            let backup_path = waterfox_db.with_extension("sqlite.cloud_reset_backup");
            std::fs::copy(&waterfox_db, &backup_path)?;
            info!("   💾 备份已创建: {:?}", backup_path);
            
            cloud_reset::clear_local_bookmarks(&waterfox_db)?;
            
            // Step 3: 等待用户同步到云端
            info!("");
            info!("📋 Step 3: 同步空书签到云端");
            cloud_reset::wait_for_cloud_sync()?;
            
            // Step 4: 验证清空
            if !cloud_reset::verify_cleared(&waterfox_db)? {
                info!("⚠️  书签可能未完全清空，但继续执行...");
            }
            
            // Step 5: 关闭Waterfox
            info!("");
            info!("📋 Step 4: 关闭Waterfox");
            let _ = std::process::Command::new("killall")
                .arg("waterfox-bin")
                .output();
            std::thread::sleep(std::time::Duration::from_secs(2));
            
            // Step 6: 执行正常同步（写入清理后的书签）
            info!("");
            info!("📋 Step 5: 写入清理后的书签");
            let mut engine = SyncEngine::new()?;
            engine.set_hub_browsers(
                "waterfox,brave-nightly",
                true,  // sync_history
                true,  // sync_reading_list
                true,  // sync_cookies
                false, // clear_others
                false, // dry_run
                false, // verbose
            ).await?;
            
            // Step 7: 提示用户再次同步
            info!("");
            info!("📋 Step 6: 同步新书签到云端");
            info!("");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("📤 请执行以下步骤：");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("");
            info!("   1. 启动 Waterfox");
            info!("   2. 等待同步图标旋转并停止（约1-2分钟）");
            info!("   3. 确认书签已恢复");
            info!("   4. 完成！云端和本地数据现在一致");
            info!("");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("");
            info!("🎉 Cloud Reset 完成！");
        }

        Commands::MigrateToSafari { yes, dry_run, keep_source, verbose } => {
            info!("🚀 Migrate to Safari - 迁移所有数据到Safari");
            info!("");
            
            if !yes && !dry_run {
                info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                info!("⚠️  警告：此操作将：");
                info!("   1. 合并所有浏览器的书签、历史、阅读列表到Safari");
                if !keep_source {
                    info!("   2. 清空其他浏览器的书签、历史、阅读列表");
                }
                info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                info!("");
                info!("使用 -y 跳过确认，或 --dry-run 预览");
                std::process::exit(0);
            }
            
            let mut engine = SyncEngine::new()?;
            engine.migrate_to_safari(dry_run, keep_source, verbose).await?;
            
            if dry_run {
                info!("✅ 预览完成（dry-run模式，未实际执行）");
            } else {
                info!("✅ 迁移完成！所有数据已迁移到Safari");
            }
        }
        
        Commands::Analyze { browsers } => {
            info!("🔍 分析书签异常...");
            let engine = SyncEngine::new()?;
            engine.analyze_bookmarks(browsers.as_deref()).await?;
        }
        
        // DeepClean命令已移除
        
        Commands::RestoreBackup { browser, file } => {
            info!("🔄 恢复书签备份...");
            let mut engine = SyncEngine::new()?;
            engine.restore_backup(&browser, file.as_deref()).await?;
            info!("✅ 备份恢复完成!");
        }
        
        Commands::MasterBackup { output, include_full } => {
            info!("📦 创建主备份...");
            sync::create_master_backup(&output, include_full).await?;
            info!("✅ 主备份创建完成!");
        }
        
        Commands::ExportHtml { output, browsers, merge, deduplicate, include_html, verbose } => {
            info!("📤 导出书签到HTML文件");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("📄 输出文件: {}", output);
            info!("🌐 来源浏览器: {}", browsers);
            if merge {
                info!("🔀 模式: 合并到单一列表");
            }
            if deduplicate {
                info!("🧹 去重复: 启用");
            }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            
            let engine = SyncEngine::new()?;
            
            // If include_html is specified, first import from HTML
            let mut extra_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
            if let Some(html_path) = &include_html {
                let expanded_path = if html_path.starts_with("~/") {
                    let home = std::env::var("HOME").unwrap_or_default();
                    html_path.replacen("~", &home, 1)
                } else {
                    html_path.clone()
                };
                
                info!("📥 导入已有HTML备份: {}", expanded_path);
                match sync::import_bookmarks_from_html(&expanded_path) {
                    Ok(bookmarks) => {
                        let count = bookmarks.iter().map(|b| count_bookmark_tree(b)).sum::<usize>();
                        info!("  ✅ 导入 {} 书签", count);
                        extra_bookmarks = bookmarks;
                    }
                    Err(e) => {
                        warn!("  ⚠️  导入失败: {}", e);
                    }
                }
            }
            
            let count = engine.export_to_html_with_extra(
                Some(&browsers),
                &output,
                merge,
                deduplicate,
                verbose,
                extra_bookmarks,
            ).await?;
            
            info!("\n🎉 导出完成! 共 {} 书签", count);
            info!("");
            info!("💡 提示: 请手动将此HTML文件导入到目标浏览器");
            info!("   这样可以避免被浏览器同步机制覆盖");
        }
        
        Commands::ClearBookmarks { browsers, yes, dry_run } => {
            info!("🗑️  清空浏览器书签");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("⚠️  警告: 此操作将清空指定浏览器的所有书签!");
            info!("🎯 目标浏览器: {}", browsers);
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            
            if !yes && !dry_run {
                print!("确认继续？(y/N): ");
                use std::io::{self, Write};
                io::stdout().flush().ok();
                
                let mut input = String::new();
                io::stdin().read_line(&mut input).ok();
                
                if !input.trim().eq_ignore_ascii_case("y") {
                    info!("❌ 已取消");
                    return Ok(());
                }
            }
            
            let mut engine = SyncEngine::new()?;
            engine.clear_bookmarks(&browsers, dry_run).await?;
            
            if dry_run {
                info!("✅ 预览完成 (dry-run模式)");
            } else {
                info!("✅ 清空完成!");
            }
        }
    }

    Ok(())
}

fn count_bookmark_tree(bookmark: &crate::browsers::Bookmark) -> usize {
    let mut count = if bookmark.url.is_some() { 1 } else { 0 };
    for child in &bookmark.children {
        count += count_bookmark_tree(child);
    }
    count
}
