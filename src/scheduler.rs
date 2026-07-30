// Scheduler module - reserved for future scheduled sync feature

use anyhow::Result;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

use crate::sync::{SyncEngine, SyncMode};

pub struct SchedulerConfig {
    pub cron_expression: String,
    pub daemon: bool,
}

impl SchedulerConfig {
    pub const fn new(cron: String, daemon: bool) -> Self {
        Self {
            cron_expression: cron,
            daemon,
        }
    }
}

pub async fn start_scheduler(config: SchedulerConfig) -> Result<()> {
    let mut scheduler = JobScheduler::new().await?;

    info!(
        "⏰ Scheduler initialized with cron: {}",
        config.cron_expression
    );

    let cron_expr = config.cron_expression.clone();
    let job = Job::new_async(cron_expr.as_str(), move |_uuid, _l| {
        Box::pin(async move {
            info!("🔄 Scheduled sync triggered");

            match SyncEngine::new() {
                Ok(mut engine) => {
                    // Use incremental mode for scheduled syncs
                    match engine.sync(SyncMode::Incremental, false, false).await {
                        Ok(stats) => {
                            info!("✅ Scheduled sync completed: {} bookmarks synced, {} duplicates removed", 
                                stats.bookmarks_synced, stats.duplicates_removed);
                        }
                        Err(e) => {
                            error!("❌ Scheduled sync failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Failed to create sync engine: {}", e);
                }
            }
        })
    })?;

    scheduler.add(job).await?;
    scheduler.start().await?;

    if config.daemon {
        info!("🔄 Running as daemon. Press Ctrl+C to stop.");
        tokio::signal::ctrl_c().await?;
        info!("🛑 Shutting down scheduler...");
    } else {
        info!("⏰ Scheduler started. Keeping process alive...");
        tokio::signal::ctrl_c().await?;
    }

    scheduler.shutdown().await?;
    Ok(())
}
