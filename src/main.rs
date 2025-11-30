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
#[command(about = "Reliable cross-browser bookmark synchronization tool with hub architecture", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Migrate all browser data to hub browsers (recommended)
    /// 
    /// This is the main command. It will:
    /// 1. Read bookmarks, history, and reading lists from ALL browsers
    /// 2. Merge and deduplicate all data
    /// 3. Write to hub browsers only
    /// 4. Optionally clear non-hub browsers
    Migrate {
        /// Hub browsers (comma-separated, e.g., "waterfox,brave-nightly")
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// Include history in migration (full history, no day limit)
        #[arg(long, default_value = "true")]
        history: bool,
        
        /// Include cookies in migration
        #[arg(long)]
        cookies: bool,
        
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
        Commands::Migrate { browsers, history, cookies, clear_others, dry_run, verbose } => {
            info!("🎯 Migrating data to hub browsers: {}", browsers);
            let mut engine = SyncEngine::new()?;
            // Always sync reading list (it's part of bookmarks for most browsers)
            engine.migrate_to_hubs(&browsers, history, cookies, clear_others, dry_run, verbose).await?;
            info!("✅ Migration complete!");
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
    }

    Ok(())
}
