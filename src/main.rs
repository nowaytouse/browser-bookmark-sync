use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

mod browsers;
mod sync;
mod scheduler;
mod validator;
mod firefox_sync;

use sync::{SyncEngine, SyncMode};
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
    Sync {
        /// Hub browsers (comma-separated)
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// Sync mode: incremental (default) or full
        #[arg(short = 'm', long, default_value = "incremental")]
        mode: String,
        
        /// Clear data from non-hub browsers
        #[arg(long)]
        clear_others: bool,
        
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
        
        /// Firefox Sync strategy: ignore, warn, trigger, or wait
        #[arg(long, default_value = "trigger")]
        firefox_sync: String,
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
    Validate {
        /// Show detailed validation report
        #[arg(short, long)]
        detailed: bool,
    },
    
    /// List all detected browsers and their bookmark locations
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
        Commands::Sync { browsers, mode, clear_others, dry_run, verbose, firefox_sync } => {
            let sync_mode = match mode.to_lowercase().as_str() {
                "incremental" | "inc" => SyncMode::Incremental,
                "full" => SyncMode::Full,
                _ => {
                    eprintln!("❌ Invalid sync mode: {}. Use 'incremental' or 'full'", mode);
                    std::process::exit(1);
                }
            };
            
            // 解析Firefox Sync策略
            let firefox_sync_strategy = match firefox_sync.to_lowercase().as_str() {
                "ignore" => firefox_sync::SyncStrategy::Ignore,
                "warn" => firefox_sync::SyncStrategy::WarnAndContinue,
                "trigger" => firefox_sync::SyncStrategy::TriggerSync,
                "wait" => firefox_sync::SyncStrategy::TriggerAndWait { timeout_secs: 60 },
                _ => {
                    eprintln!("❌ Invalid firefox-sync strategy: {}. Use 'ignore', 'warn', 'trigger', or 'wait'", firefox_sync);
                    std::process::exit(1);
                }
            };
            
            info!("🔄 Starting {:?} sync between hub browsers: {}", sync_mode, browsers);
            let mut engine = SyncEngine::new()?;
            // Full sync: bookmarks + history + reading list + cookies
            engine.set_hub_browsers_with_firefox_sync(
                &browsers, 
                true, 
                true, 
                true, 
                clear_others, 
                dry_run, 
                verbose,
                firefox_sync_strategy
            ).await?;
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
    }

    Ok(())
}
