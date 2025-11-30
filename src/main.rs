use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{info, error};

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
    }

    Ok(())
}
