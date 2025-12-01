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

#[derive(Parser)]
#[command(name = "browser-bookmark-sync")]
#[command(about = "Cross-browser bookmark management tool - merge, deduplicate, export", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all detected browsers and their bookmark locations
    #[command(alias = "l", alias = "ls")]
    List,
    
    /// Export bookmarks to HTML file (RECOMMENDED - avoids sync conflicts)
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
        
        /// Remove empty folders before export
        #[arg(long)]
        clean_empty: bool,
        
        /// Also import from existing HTML backup file
        #[arg(long)]
        include_html: Option<String>,
        
        /// Clear bookmarks from source browsers after export (WARNING: destructive!)
        #[arg(long)]
        clear_after: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Validate bookmark integrity
    #[command(alias = "v", alias = "check")]
    Validate {
        /// Show detailed validation report
        #[arg(short, long)]
        detailed: bool,
    },
    
    /// Clean up bookmarks (remove duplicates/empty folders) - MODIFIES BROWSER DATA
    #[command(alias = "c", alias = "clean")]
    Cleanup {
        /// Target browsers (comma-separated, default: all)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// Remove duplicate bookmarks
        #[arg(long)]
        remove_duplicates: bool,
        
        /// Remove empty folders
        #[arg(long)]
        remove_empty_folders: bool,
        
        /// Dry run - preview without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Smart organize bookmarks by URL patterns - MODIFIES BROWSER DATA
    #[command(alias = "so", alias = "smart")]
    SmartOrganize {
        /// Target browsers (comma-separated, default: all)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// Custom rules file (JSON format)
        #[arg(short = 'r', long)]
        rules_file: Option<String>,
        
        /// Only process uncategorized bookmarks
        #[arg(long)]
        uncategorized_only: bool,
        
        /// Show rule matching statistics
        #[arg(long)]
        show_stats: bool,
        
        /// Dry run - preview without making changes
        #[arg(short, long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// List available classification rules
    ListRules,
    
    /// Sync browsing history between hub browsers
    #[command(alias = "sh", alias = "history")]
    SyncHistory {
        /// Hub browsers (comma-separated, default: waterfox,brave-nightly)
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// Number of days to sync (default: 30)
        #[arg(short = 'd', long, default_value = "30")]
        days: i32,
        
        /// Dry run - preview without making changes
        #[arg(long)]
        dry_run: bool,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Analyze bookmarks (NSFW detection, duplicates)
    #[command(alias = "a")]
    Analyze {
        /// Target browsers (comma-separated, default: all)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
    },
    
    /// Create master backup (merge all browser data)
    MasterBackup {
        /// Output directory
        #[arg(short = 'o', long, default_value = "~/Desktop/BookmarkBackup")]
        output: String,
        
        /// Include full data (not just unique URLs)
        #[arg(long)]
        include_full: bool,
    },
    
    /// Restore bookmarks from backup
    RestoreBackup {
        /// Browser to restore (e.g., waterfox)
        #[arg(short = 'b', long)]
        browser: String,
        
        /// Backup file path (optional, uses latest if not specified)
        #[arg(short = 'f', long)]
        file: Option<String>,
    },
    
    /// Clear all bookmarks from browsers (DEBUG ONLY - use with caution!)
    #[command(alias = "clear")]
    ClearBookmarks {
        /// Target browsers (comma-separated)
        #[arg(short = 'b', long)]
        browsers: String,
        
        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
        
        /// Dry run - preview without making changes
        #[arg(short, long)]
        dry_run: bool,
    },
}

/// Print sync warning for operations that modify browser data
fn print_sync_warning() {
    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    warn!("⚠️  WARNING: This operation modifies browser bookmark data!");
    warn!("");
    warn!("   If browser sync is enabled (Firefox Sync, Chrome Sync, iCloud, etc.),");
    warn!("   changes may be overwritten or cause unexpected sync conflicts.");
    warn!("");
    warn!("   RECOMMENDED: Use 'export-html' instead and import manually.");
    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            info!("Detecting browsers...");
            let engine = SyncEngine::new()?;
            engine.list_browsers()?;
        }
        
        Commands::ExportHtml { output, browsers, merge, deduplicate, clean_empty, include_html, clear_after, verbose } => {
            info!("Exporting bookmarks to HTML");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("Output: {}", output);
            info!("Source: {}", browsers);
            if merge { info!("Mode: Merged (flat structure)"); }
            if deduplicate { info!("Deduplicate: Yes"); }
            if clean_empty { info!("Clean empty folders: Yes"); }
            if clear_after { 
                warn!("Clear after: YES (will delete original bookmarks!)"); 
            }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            
            let mut engine = SyncEngine::new()?;
            
            let mut extra_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
            if let Some(html_path) = &include_html {
                let expanded = if html_path.starts_with("~/") {
                    html_path.replacen("~", &std::env::var("HOME").unwrap_or_default(), 1)
                } else { html_path.clone() };
                
                info!("Importing HTML: {}", expanded);
                match sync::import_bookmarks_from_html(&expanded) {
                    Ok(bookmarks) => {
                        let count = bookmarks.iter().map(|b| count_bookmark_tree(b)).sum::<usize>();
                        info!("  Imported {} bookmarks", count);
                        extra_bookmarks = bookmarks;
                    }
                    Err(e) => warn!("  Import failed: {}", e),
                }
            }
            
            let count = engine.export_to_html_with_extra(
                Some(&browsers), &output, merge, deduplicate, clean_empty, verbose, extra_bookmarks
            ).await?;
            
            info!("");
            info!("Export complete! {} bookmarks", count);
            
            // Clear bookmarks after export if requested
            if clear_after {
                warn!("");
                warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                warn!("⚠️  WARNING: Clearing bookmarks from source browsers!");
                warn!("");
                warn!("   If browser sync is enabled (Firefox Sync, Chrome Sync, iCloud, etc.),");
                warn!("   deletion may be ineffective or cause unpredictable bookmark versions.");
                warn!("   Consider disabling sync before using this option.");
                warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                
                engine.clear_bookmarks(&browsers, false).await?;
                info!("✅ Source bookmarks cleared. Import the HTML file to restore.");
            } else {
                info!("Import this file manually to avoid sync conflicts.");
            }
        }
        
        Commands::Validate { detailed } => {
            info!("Validating bookmarks...");
            let engine = SyncEngine::new()?;
            let report = engine.validate(detailed)?;
            println!("{}", report);
        }
        
        Commands::Cleanup { browsers, remove_duplicates, remove_empty_folders, dry_run, verbose } => {
            if !remove_duplicates && !remove_empty_folders {
                eprintln!("Error: Specify --remove-duplicates or --remove-empty-folders");
                std::process::exit(1);
            }
            
            if !dry_run {
                print_sync_warning();
            }
            
            info!("Cleaning up bookmarks...");
            let mut engine = SyncEngine::new()?;
            engine.cleanup_bookmarks(
                browsers.as_deref(), remove_duplicates, remove_empty_folders, dry_run, verbose
            ).await?;
            info!("Cleanup complete!");
        }
        
        Commands::SmartOrganize { browsers, rules_file, uncategorized_only, show_stats, dry_run, verbose } => {
            if !dry_run {
                print_sync_warning();
            }
            
            info!("Smart organizing bookmarks...");
            let mut engine = SyncEngine::new()?;
            engine.smart_organize(
                browsers.as_deref(), rules_file.as_deref(), uncategorized_only, show_stats, dry_run, verbose
            ).await?;
            info!("Organization complete!");
        }
        
        Commands::ListRules => {
            SyncEngine::print_builtin_rules();
        }
        
        Commands::SyncHistory { browsers, days, dry_run, verbose } => {
            info!("Syncing browser history");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("Hub browsers: {}", browsers);
            info!("Sync range: Last {} days", days);
            if dry_run { info!("Mode: Dry run"); }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            
            let mut engine = SyncEngine::new()?;
            engine.sync_history(Some(days), dry_run, verbose).await?;
            info!("History sync complete!");
        }
        
        Commands::Analyze { browsers } => {
            info!("Analyzing bookmarks...");
            let engine = SyncEngine::new()?;
            engine.analyze_bookmarks(browsers.as_deref()).await?;
        }
        
        Commands::MasterBackup { output, include_full } => {
            info!("Creating master backup...");
            sync::create_master_backup(&output, include_full).await?;
            info!("Backup complete!");
        }
        
        Commands::RestoreBackup { browser, file } => {
            print_sync_warning();
            info!("Restoring backup...");
            let mut engine = SyncEngine::new()?;
            engine.restore_backup(&browser, file.as_deref()).await?;
            info!("Restore complete!");
        }
        
        Commands::ClearBookmarks { browsers, yes, dry_run } => {
            warn!("WARNING: This will clear ALL bookmarks!");
            warn!("Target: {}", browsers);
            
            if !dry_run {
                print_sync_warning();
            }
            
            if !yes && !dry_run {
                print!("Confirm? (y/N): ");
                use std::io::{self, Write};
                io::stdout().flush().ok();
                let mut input = String::new();
                io::stdin().read_line(&mut input).ok();
                if !input.trim().eq_ignore_ascii_case("y") {
                    info!("Cancelled");
                    return Ok(());
                }
            }
            
            let mut engine = SyncEngine::new()?;
            engine.clear_bookmarks(&browsers, dry_run).await?;
            info!("Clear complete!");
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
