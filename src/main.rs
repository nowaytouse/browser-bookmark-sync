use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

mod browsers;
mod sync;
mod scheduler;
mod validator;

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
    /// Perform a one-time sync across all browsers
    Sync {
        /// Dry run - show what would be synced without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
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
    
    /// Synchronize browsing history across browsers
    SyncHistory {
        /// Only sync history from last N days
        #[arg(short, long)]
        days: Option<i32>,
        
        /// Dry run - show what would be synced without making changes
        #[arg(short = 'n', long)]
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
    
    /// Set hub browsers and clean others (migrate data to hubs, then clear non-hubs)
    SetHubs {
        /// Hub browsers (comma-separated, e.g., "waterfox,brave-nightly")
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// Sync history to hubs
        #[arg(long)]
        sync_history: bool,
        
        /// Sync reading list to hubs
        #[arg(long)]
        sync_reading_list: bool,
        
        /// Sync cookies to hubs
        #[arg(long)]
        sync_cookies: bool,
        
        /// Clear bookmarks from non-hub browsers after migration
        #[arg(long)]
        clear_others: bool,
        
        /// Dry run - show what would be done without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
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
        Commands::Sync { dry_run, verbose } => {
            info!("🔄 Starting bookmark synchronization...");
            let mut engine = SyncEngine::new()?;
            engine.sync(dry_run, verbose).await?;
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
        
        Commands::SyncHistory { days, dry_run, verbose } => {
            info!("📜 Starting history synchronization...");
            let mut engine = SyncEngine::new()?;
            engine.sync_history(days, dry_run, verbose).await?;
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
        
        Commands::SetHubs { browsers, sync_history, sync_reading_list, sync_cookies, clear_others, dry_run, verbose } => {
            info!("🎯 Setting hub browsers: {}", browsers);
            let mut engine = SyncEngine::new()?;
            engine.set_hub_browsers(&browsers, sync_history, sync_reading_list, sync_cookies, clear_others, dry_run, verbose).await?;
            info!("✅ Hub configuration complete!");
        }
    }

    Ok(())
}
