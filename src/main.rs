use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

mod browser_utils;
mod browsers;
mod cleanup;
mod cloud_reset;
mod db_safety;
mod firefox_sync;
mod firefox_sync_api;
mod scheduler;
mod sync;
mod sync_flags;
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

    /// Create full backup of all browser data
    Backup {
        /// Output directory
        #[arg(short, long, default_value = "~/Desktop/BookmarkBackup")]
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

        Commands::Backup { output } => {
            info!("💾 Creating backup...");
            sync::create_master_backup(&output, true).await?;
            info!("✅ Backup complete: {}", output);
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
