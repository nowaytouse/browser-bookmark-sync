#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic,))]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

use browser_bookmark_sync::{
    browser_utils, browsers, data_types, expand_path, gui,
    health::{
        build_probe_plan, models::BackendObservation, reporting::HealthRunSummaryReport,
        reporting::ReportExporter, BookmarkTargetInput, DnsPolicy, HealthEngine,
        HealthEngineOptions, HealthVerdict, ProbePlan, ProbeScope, ProbeTarget, SkippedTarget,
        TrustedSiteSkipList,
    },
    sync::{self, SyncEngine},
    sync_flags::SyncFlags,
    url_checker,
};

fn parse_positive_usize(value: &str) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer: {error}"))?;
    if parsed == 0 {
        Err("value must be at least 1".to_string())
    } else {
        Ok(parsed)
    }
}

fn parse_probe_scope(value: &str) -> std::result::Result<ProbeScope, String> {
    match value {
        "site" => Ok(ProbeScope::Site),
        "content" => Ok(ProbeScope::Content),
        _ => Err("scope must be site or content".to_string()),
    }
}

fn is_disabled_non_core_command(command: &Commands) -> bool {
    matches!(
        command,
        Commands::History { .. }
            | Commands::ExportData { .. }
            | Commands::Gui { .. }
            | Commands::Organize { .. }
            | Commands::Pipeline { .. }
            | Commands::WaterfoxCleanup { .. }
    )
}

const DEFAULT_TRUSTED_SITE_SKIP_LIST: &str = include_str!("../trusted_site_skip_list.json");

const MAX_HEALTH_CONCURRENCY: usize = 128;

fn build_log_filter() -> Result<tracing_subscriber::EnvFilter> {
    Ok(tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into())
        .add_directive("html5ever=error".parse()?))
}

fn init_logging(log_path: Option<&std::path::Path>) -> Result<()> {
    if let Some(path) = log_path {
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "unable to create log directory {}: {error}",
                    parent.display()
                );
            }
        }
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(file) => {
                use tracing_subscriber::fmt::writer::MakeWriterExt;
                tracing_subscriber::fmt()
                    .with_env_filter(build_log_filter()?)
                    .with_writer(std::io::stderr.and(file))
                    .init();
                return Ok(());
            }
            Err(error) => {
                eprintln!("unable to open log file {}: {error}", path.display());
            }
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(build_log_filter()?)
        .init();
    Ok(())
}

fn default_check_log_path(cli: &Cli) -> Option<std::path::PathBuf> {
    if let Some(path) = cli.log_file.as_deref() {
        return Some(std::path::PathBuf::from(expand_path(path)));
    }
    match &cli.command {
        Commands::Check {
            export_dir: Some(dir),
            ..
        } => Some(std::path::PathBuf::from(expand_path(dir)).join("browser-sync.log")),
        Commands::Check { .. } => Some(std::path::PathBuf::from("./browser-sync-check.log")),
        _ => None,
    }
}

struct CheckStatusGuard {
    path: Option<std::path::PathBuf>,
    scope: &'static str,
    input: Option<String>,
    started_at: String,
    finished: bool,
}

impl CheckStatusGuard {
    fn start(export_dir: Option<&str>, scope: ProbeScope, input: Option<&str>) -> Self {
        let path = export_dir
            .map(|dir| std::path::PathBuf::from(expand_path(dir)).join("check-status.json"));
        let guard = Self {
            path,
            scope: match scope {
                ProbeScope::Site => "site",
                ProbeScope::Content => "content",
            },
            input: input.map(str::to_string),
            started_at: chrono::Utc::now().to_rfc3339(),
            finished: false,
        };
        guard.write("running");
        info!(
            event = "check_status_started",
            pid = std::process::id(),
            scope = guard.scope,
            status_file = ?guard.path,
            "check status tracking started"
        );
        guard
    }

    fn write(&self, status: &str) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let payload = serde_json::json!({
            "schema_version": 1,
            "pid": std::process::id(),
            "status": status,
            "scope": self.scope,
            "input": self.input,
            "started_at": self.started_at,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "unable to create status directory {}: {error}",
                    parent.display()
                );
                return;
            }
        }
        if let Err(error) = browser_utils::atomic_write_file(path, payload.to_string().as_bytes()) {
            eprintln!("unable to write status file {}: {error}", path.display());
        }
    }

    fn finish(&mut self) {
        self.finished = true;
        self.write("completed");
        info!(
            event = "check_status_completed",
            pid = std::process::id(),
            scope = self.scope,
            status_file = ?self.path,
            "check status tracking completed"
        );
    }
}

impl Drop for CheckStatusGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.write("aborted");
            warn!(
                event = "check_status_aborted",
                pid = std::process::id(),
                scope = self.scope,
                status_file = ?self.path,
                "check status tracking aborted before normal completion"
            );
        }
    }
}

#[cfg(unix)]
async fn wait_for_termination_signal() -> Result<&'static str> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    tokio::select! {
        value = sigterm.recv() => value.context("SIGTERM signal stream closed").map(|_| "SIGTERM"),
        value = sigint.recv() => value.context("SIGINT signal stream closed").map(|_| "SIGINT"),
    }
}

#[cfg(not(unix))]
async fn wait_for_termination_signal() -> Result<&'static str> {
    tokio::signal::ctrl_c()
        .await
        .context("install Ctrl-C handler")?;
    Ok("CTRL_C")
}

const fn bounded_health_concurrency(requested: usize) -> usize {
    if requested > MAX_HEALTH_CONCURRENCY {
        MAX_HEALTH_CONCURRENCY
    } else {
        requested
    }
}

#[derive(Parser)]
#[command(name = "bsync")]
#[command(
    about = "Cross-browser bookmark cleanup: collect all profiles, deduplicate, check health, quarantine"
)]
#[command(version)]
#[command(after_help = "EXAMPLES:
    bsync inventory                          # Discover every browser profile
    bsync export -d --merge                 # Export and exactly deduplicate bookmarks
    bsync check --file ./bookmarks.html --slow --history-db ./bookmark-health.sqlite
    bsync health quarantine --db ./bookmark-health.sqlite")]
struct Cli {
    /// Append structured process and progress logs to this file.
    #[arg(long, global = true)]
    log_file: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect and audit installed browsers and profiles without writing HTML
    Inventory {
        /// Include all profiles of each browser
        #[arg(short = 'a', long, default_value = "true")]
        all_profiles: bool,
    },

    /// Collect original un-deduplicated bookmark tree and export inventory
    Collect {
        /// Output directory for original HTML and inventory
        #[arg(short, long, default_value = "./bookmark-run")]
        output_dir: String,

        /// Preserve original source tree hierarchy
        #[arg(long, default_value = "true")]
        preserve_source_tree: bool,
    },

    /// Perform exact deduplication over harvested inventory or bookmark tree
    Dedupe {
        /// Output directory for deduplicated HTML and audit reports
        #[arg(short, long, default_value = "./bookmark-run")]
        output_dir: String,

        /// Deduplication mode (exact)
        #[arg(long, default_value = "exact")]
        mode: String,
    },

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
        #[arg(long, hide = true)]
        history: bool,

        /// Include reading list (Safari, Firefox)
        #[arg(short = 'r', long)]
        reading_list: bool,

        /// Include cookies (⚠️  affects sessions)
        #[arg(long, hide = true)]
        cookies: bool,

        /// Include passwords (⚠️  SECURITY RISK - BLOCKED)
        #[arg(long, hide = true)]
        passwords: bool,

        /// Include extensions (⚠️  NOT SUPPORTED - BLOCKED)
        #[arg(long, hide = true)]
        extensions: bool,

        /// Days of history to export (default: 30, 0 = all)
        #[arg(long, default_value = "30", hide = true)]
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

        /// Import from existing HTML files, SQLite databases, or external profile directories
        #[arg(long, num_args = 0..)]
        include: Vec<String>,

        /// Exclude URLs whose hostname is onion or ends in .onion before aggregation
        #[arg(long, default_value = "true")]
        exclude_onion: bool,

        /// Clear source browsers after export (⚠️  DANGEROUS!)
        #[arg(long, hide = true)]
        clear_after: bool,

        /// Enable unsafe database writes (required for `clear_after`)
        #[arg(long, hide = true)]
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

        /// Copy exported bookmark HTML to a copy path and perform validity checking
        #[arg(long)]
        validate_copy: Option<String>,

        /// Enable content-level validity checking (checks specific page/video URLs for 404/410 instead of just apex domain)
        #[arg(long)]
        validate_content: bool,

        /// Concurrency limit for validity checking (default: 10, max: 10)
        #[arg(
            long,
            default_value = "10",
            value_parser = parse_positive_usize
        )]
        validate_concurrency: usize,

        /// Limit number of URLs to check during copy validation (default: 0 = all)
        #[arg(long, default_value = "0")]
        validate_limit: usize,
    },

    /// Analyze bookmarks (duplicates, empty folders, NSFW)
    #[command(alias = "a")]
    #[command(hide = true)]
    Analyze {
        /// Target browsers
        #[arg(short, long)]
        browsers: Option<String>,
    },

    /// Smart organize bookmarks by URL patterns
    #[command(alias = "org", alias = "o")]
    #[command(hide = true)]
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
        #[arg(short, long, required = true)]
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

        /// Template/rules backend (only `rules` is supported)
        #[arg(long, default_value = "rules", hide = true)]
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
    #[command(hide = true)]
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
    #[command(hide = true)]
    Rules,

    /// Check bookmark URL validity (dual-network validation)
    #[command(alias = "c", alias = "chk")]
    Check {
        /// Input bookmark file (HTML) - check from exported file instead of browser
        #[arg(short, long)]
        file: Option<String>,

        /// Separate output path used for the cleaned HTML copy
        #[arg(short, long)]
        output: Option<String>,

        /// Proxy server URL (e.g., <http://127.0.0.1:7890>)
        #[arg(short, long)]
        proxy: Option<String>,

        /// Request timeout in seconds
        #[arg(
            short,
            long,
            default_value = "10",
            value_parser = clap::value_parser!(u64).range(1..)
        )]
        timeout: u64,

        /// Number of concurrent requests (max 128; each registrable domain remains bounded)
        #[arg(
            short,
            long,
            default_value = "5",
            value_parser = parse_positive_usize
        )]
        concurrency: usize,

        /// Delete confirmed invalid bookmarks
        #[arg(long, hide = true)]
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

        /// Limit number of URLs to check (0 = check all)
        #[arg(short = 'L', long, default_value = "0")]
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

        /// Enable slow conservative probing mode
        #[arg(long)]
        slow: bool,

        /// Path to SQLite health history database
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        history_db: Option<String>,

        /// Secondary probe backends (comma separated, e.g., curl,lychee,httpx)
        #[arg(long, default_value = "curl")]
        secondary: Option<String>,

        /// Probe scope: site groups URLs by registrable domain; content checks full URLs
        #[arg(long, value_parser = parse_probe_scope, default_value = "content")]
        scope: ProbeScope,

        /// Optional trusted-site skip-list JSON path (site scope only)
        #[arg(long)]
        trusted_site_skip_list: Option<String>,

        /// Disable trusted-site skipping in site scope
        #[arg(long)]
        no_trusted_site_skip: bool,

        /// Optional DNS resolver policy JSON path
        #[arg(long)]
        dns_policy: Option<String>,
    },

    /// Manage multi-run bookmark health history, audit tools, and quarantine reports
    Health {
        #[command(subcommand)]
        command: browser_bookmark_sync::health::HealthCommands,
    },

    /// Create full backup of all browser data
    #[command(hide = true)]
    Backup {
        /// Output directory
        #[arg(short, long, default_value = "~/Desktop/BookmarkBackup")]
        output: String,
    },

    /// Export sensitive browser data (passwords, cookies, downloads)
    #[command(alias = "data")]
    #[command(hide = true)]
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

    /// Run hidden export -> health check pipeline without organization or browser writes
    #[command(alias = "full-process", alias = "process", alias = "pipe")]
    #[command(hide = true)]
    Pipeline {
        /// Output directory path
        #[arg(short, long, default_value = "~/Downloads/pipeline_results")]
        output_dir: String,

        /// Source browsers (comma-separated, or 'all')
        #[arg(short, long, default_value = "all")]
        browsers: String,

        /// Include extra HTML files, SQLite databases, or external profile directories
        #[arg(short, long, num_args = 0..)]
        include: Vec<String>,

        /// Proxy URL for dead link checking
        #[arg(short, long)]
        proxy: Option<String>,

        /// Check limit (0 = check all)
        #[arg(short, long, default_value = "0")]
        limit: usize,

        /// Concurrent check tasks
        #[arg(
            short = 'c',
            long,
            default_value = "10",
            value_parser = parse_positive_usize
        )]
        concurrency: usize,

        /// HTTP timeout in seconds
        #[arg(
            short,
            long,
            default_value = "15",
            value_parser = clap::value_parser!(u64).range(1..)
        )]
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
    #[command(hide = true)]
    WaterfoxCleanup {
        /// Output file path
        #[arg(short, long, default_value = "~/Downloads/waterfox_cleaned.html")]
        output: String,
    },

    /// Launch local web GUI in browser (default: <http://127.0.0.1:7890>)
    #[command(alias = "ui", alias = "dashboard")]
    #[command(hide = true)]
    Gui {
        /// Port number for the local web server
        #[arg(short, long, default_value = "7890")]
        port: u16,
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
    let cli = Cli::parse();
    let log_path = default_check_log_path(&cli);
    init_logging(log_path.as_deref())?;
    info!(
        event = "process_started",
        pid = std::process::id(),
        log_file = ?log_path,
        "browser bookmark sync process started"
    );

    tokio::select! {
        // Register termination handlers before check setup performs synchronous work.
        biased;
        signal = wait_for_termination_signal() => {
            let signal = signal?;
            warn!(
                event = "process_terminated",
                pid = std::process::id(),
                signal,
                "browser bookmark sync process received a termination signal"
            );
            Err(anyhow::anyhow!("process terminated by {signal}"))
        }
        result = run_cli(cli) => {
            match &result {
                Ok(()) => info!(
                    event = "process_finished",
                    pid = std::process::id(),
                    "browser bookmark sync process finished"
                ),
                Err(error) => error!(
                    event = "process_failed",
                    pid = std::process::id(),
                    %error,
                    "browser bookmark sync process failed"
                ),
            }
            result
        }
    }
}

async fn run_cli(cli: Cli) -> Result<()> {
    if is_disabled_non_core_command(&cli.command) {
        anyhow::bail!(
            "This legacy command is disabled: use the core collect/export/check/quarantine workflow; history, organization, legacy pipelines, sensitive export, and GUI are unsupported"
        );
    }

    match cli.command {
        Commands::Inventory { .. } => {
            use sync::collector::BookmarkCollector;
            let installations = BookmarkCollector::discover_installations()?;
            info!(
                "🔍 Discovered {} browser installation(s)",
                installations.len()
            );
            for inst in &installations {
                let profiles = BookmarkCollector::discover_profiles(inst)?;
                info!(
                    "  🌐 {} ({} profile(s)): {:?}",
                    inst.name,
                    profiles.len(),
                    profiles.iter().map(|p| &p.name).collect::<Vec<_>>()
                );
            }
        }

        Commands::Collect { output_dir, .. } => {
            use sync::collector::BookmarkCollector;
            use sync::exporter::ReportExporter;
            let out_path = std::path::PathBuf::from(&output_dir);
            std::fs::create_dir_all(&out_path)?;
            let (tree, inventory, manifest) = BookmarkCollector::collect_all(&out_path)?;

            let orig_html_path = out_path.join("bookmarks-original.html");
            ReportExporter::export_original_html(&tree, &orig_html_path)?;
            ReportExporter::write_sidecar_reports(&out_path, &inventory, &[], &[], &manifest)?;

            info!(
                "✅ Collection complete! Exported original HTML: {:?}",
                orig_html_path
            );
            info!("✅ Sidecar inventory & manifest written to {:?}", out_path);
        }

        Commands::Dedupe { output_dir, .. } => {
            use sync::collector::BookmarkCollector;
            use sync::dedupe::DedupeEngine;
            use sync::exporter::ReportExporter;
            let out_path = std::path::PathBuf::from(&output_dir);
            std::fs::create_dir_all(&out_path)?;

            let (tree, inventory, mut manifest) = BookmarkCollector::collect_all(&out_path)?;
            let (canonicals, reports, csv_records, total_raw, total_dups) =
                DedupeEngine::deduplicate_tree(&tree, &[]);

            manifest.total_canonical_bookmarks = canonicals.len();
            manifest.total_removed_duplicates = total_dups;
            manifest.invariant_satisfied = total_raw == canonicals.len() + total_dups;

            let dedup_html_path = out_path.join("bookmarks-deduplicated.html");
            ReportExporter::export_deduplicated_html(&canonicals, &dedup_html_path)?;
            ReportExporter::write_sidecar_reports(
                &out_path,
                &inventory,
                &reports,
                &csv_records,
                &manifest,
            )?;

            info!("✅ Deduplication complete! Output: {:?}", dedup_html_path);
            info!(
                "📊 Total raw: {}, Canonical kept: {}, Duplicates merged: {}",
                total_raw,
                canonicals.len(),
                total_dups
            );
            info!(
                "🔒 Invariant check (Raw == Canonical + Merged): {}",
                manifest.invariant_satisfied
            );
        }
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
            exclude_onion,
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
            validate_copy,
            validate_content,
            validate_concurrency,
            validate_limit,
        } => {
            if history || cookies || passwords || extensions || clear_after || unsafe_write {
                println!(
                    "Non-bookmark export options are disabled: history, cookies, passwords, extensions, and browser write-back are unavailable."
                );
                return Ok(());
            }

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
            if exclude_onion {
                info!("  ✓ Exclude onion hostnames before aggregation");
            }
            if clear_after {
                warn!("  ⚠️  Clear after export");
            }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            let mut engine = SyncEngine::new()?;

            // Import from existing HTML files, SQLite files, or external profile directories
            let mut extra_bookmarks: Vec<browsers::Bookmark> = Vec::new();
            let mut external_source_reports: Vec<serde_json::Value> = Vec::new();
            for source_path in &include {
                let expanded = expand_path(source_path);
                info!("📥 导入外部数据源: {}", expanded);
                match sync::import_bookmarks_from_source_with_report(&expanded) {
                    Ok((mut bookmarks, mut report)) => {
                        let excluded_onion = if exclude_onion {
                            sync::remove_onion_bookmarks(&mut bookmarks)
                        } else {
                            0
                        };
                        let count: usize = bookmarks.iter().map(count_tree).sum();
                        info!("   ✅ 成功从 '{}' 导入 {} 条书签节点", expanded, count);
                        if excluded_onion > 0 {
                            info!("   🧅 已排除 {excluded_onion} 条 onion 书签");
                        }
                        if let Some(report) = report.as_object_mut() {
                            report.insert(
                                "excluded_onion_bookmarks".into(),
                                serde_json::json!(excluded_onion),
                            );
                            report.insert(
                                "bookmark_count_after_filters".into(),
                                serde_json::json!(count),
                            );
                        }
                        external_source_reports.push(report);
                        extra_bookmarks.extend(bookmarks);
                    }
                    Err(e) => {
                        error!("❌ 无法从外部数据源 '{}' 导入书签: {}", expanded, e);
                        // Keep the remaining sources usable, but make the failure explicit in
                        // the export report so a partial input can never look complete.
                        external_source_reports.push(serde_json::json!({
                            "source_path": expanded,
                            "source_type": "unreadable",
                            "status": "failed",
                            "error": e.to_string(),
                            "bookmark_count": 0,
                            "folder_count": 0,
                        }));
                    }
                }
            }

            // Include Safari reading list if requested
            if reading_list {
                info!("📖 Reading Safari reading list...");
                match engine.get_safari_reading_list() {
                    Ok(items) if !items.is_empty() => {
                        info!("   {} items found", items.len());
                        let reading_folder = browsers::Bookmark {
                            id: "reading-list".to_string(),
                            title: "Reading List".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .map(|item| browsers::Bookmark {
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
                        let history_folder = browsers::Bookmark {
                            id: "history".to_string(),
                            title: "History".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .enumerate()
                                .map(|(i, item)| browsers::Bookmark {
                                    id: format!("hist-{i}"),
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
                        let cookies_folder = browsers::Bookmark {
                            id: "cookies".to_string(),
                            title: "Cookies".to_string(),
                            url: None,
                            folder: true,
                            children: items
                                .into_iter()
                                .enumerate()
                                .map(|(i, item)| browsers::Bookmark {
                                    id: format!("cookie-{i}"),
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
                exclude_onion,
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
                if exclude_onion {
                    let removed = sync::remove_onion_bookmarks(&mut existing_bookmarks);
                    if removed > 0 {
                        info!("🧅 Removed {removed} onion bookmark(s) from update input");
                    }
                }
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
                    .export_to_html_with_report(
                        Some(&browsers),
                        &output,
                        None,
                        &export_config,
                        extra_bookmarks,
                        external_source_reports,
                    )
                    .await?
            };

            info!("");
            info!("✅ Exported {} bookmarks to {}", count, output);

            if let Some(ref copy_path) = validate_copy {
                let expanded_copy = expand_path(copy_path);
                let expanded_output = expand_path(&output);
                info!(
                    "📋 Copying exported file to {} for validation...",
                    expanded_copy
                );
                if let Err(e) = std::fs::copy(&expanded_output, &expanded_copy) {
                    error!("❌ Failed to copy file for validation: {}", e);
                } else {
                    info!("✅ Copied to {}. Running validity check...", expanded_copy);
                    use std::collections::HashSet;
                    use url_checker::{collect_urls_from_bookmarks, CheckerConfig, UrlChecker};
                    if let Ok(bookmarks) = sync::import_bookmarks_from_html(&expanded_copy) {
                        let all_urls: HashSet<String> = collect_urls_from_bookmarks(&bookmarks)
                            .into_iter()
                            .collect();
                        let safe_conc = validate_concurrency.min(10);
                        let cfg = CheckerConfig {
                            proxy_url: None,
                            timeout_secs: 10,
                            concurrency: safe_conc,
                            retry_count: 1,
                        };
                        if let Ok(checker) = UrlChecker::new(cfg) {
                            use indicatif::{ProgressBar, ProgressStyle};
                            let mut url_vec: Vec<String> = all_urls.into_iter().collect();
                            if validate_limit > 0 && url_vec.len() > validate_limit {
                                info!(
                                    "📊 限制有效性检查数量: {} / {} URLs",
                                    validate_limit,
                                    url_vec.len()
                                );
                                url_vec.truncate(validate_limit);
                            }
                            let pb = ProgressBar::new(url_vec.len() as u64);
                            pb.set_style(
                                ProgressStyle::default_bar()
                                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                                    .unwrap_or_else(|_| ProgressStyle::default_bar())
                                    .progress_chars("#>-"),
                            );
                            let pb2 = pb.clone();
                            let results = if validate_content {
                                info!("🎯 启用【内容有效性检测 (Content Validity Mode)】: 针对主域名有效的前提下，深度排查具体页面内容是否返回 HTTP 404 Not Found / 410 Gone...");
                                checker
                                    .check_batch_content(url_vec, move |current, total, _url| {
                                        pb2.set_length(total as u64);
                                        pb2.set_position(current as u64);
                                    })
                                    .await
                            } else {
                                info!("🌐 启用【网站主域名去重有效性检测 (Domain Validity Mode)】: 结合大站白名单豁免，探测主域可达性...");
                                let pb3 = pb.clone();
                                checker
                                    .check_batch(url_vec, move |current, total, _url| {
                                        pb3.set_length(total as u64);
                                        pb3.set_position(current as u64);
                                    })
                                    .await
                            };
                            pb.finish_with_message("检查完成");
                            let total_checked = results.len();
                            let review_candidate_urls: HashSet<String> = results
                                .iter()
                                .filter(|r| r.status.should_auto_delete())
                                .map(|r| r.url.clone())
                                .collect();
                            let exempt_count =
                                results.iter().filter(|r| r.url.contains(".onion")).count();
                            let valid_count = results
                                .iter()
                                .filter(|r| {
                                    matches!(
                                        r.status,
                                        url_checker::ValidationStatus::Valid
                                            | url_checker::ValidationStatus::Redirected
                                    )
                                })
                                .count();
                            let doubtful_count = total_checked
                                .saturating_sub(review_candidate_urls.len() + valid_count);

                            let review_candidate_items: Vec<serde_json::Value> = results
                                .iter()
                                .filter(|r| r.status.should_auto_delete())
                                .map(|r| {
                                    serde_json::json!({
                                        "url": r.url,
                                        "status": format!("{:?}", r.status),
                                        "reason": r.error_message.clone().unwrap_or_else(|| "单轮检查命中失效候选 (DNS无法解析 / 服务器主动拒绝连接 / 404 Not Found / 410 Gone)".to_string()),
                                        "action": "候选待复核，校验副本保留 (Review candidate; validation copy retained)"
                                    })
                                })
                                .collect();

                            let doubtful_retained_items: Vec<serde_json::Value> = results
                                .iter()
                                .filter(|r| {
                                    !r.status.should_auto_delete()
                                        && !matches!(
                                            r.status,
                                            url_checker::ValidationStatus::Valid
                                                | url_checker::ValidationStatus::Redirected
                                        )
                                })
                                .map(|r| {
                                    serde_json::json!({
                                        "url": r.url,
                                        "status": format!("{:?}", r.status),
                                        "reason": r.error_message.clone().unwrap_or_else(|| "存疑保留 (本地网络与代理IO临时错误 / 网站暂时维护5xx / 403防护与429限流 / 暗网地址)".to_string()),
                                        "action": "存疑但保留 (Retained: local network IO error or temporary server maintenance)"
                                    })
                                })
                                .collect();

                            let validity_conclusions = serde_json::json!({
                                "validation_performed": true,
                                "validation_mode": if validate_content {
                                    "ContentValidity (内容有效性检测 - 针对主域名有效的前提下，排查具体完整页面路径返回 404/410 的失效候选)"
                                } else {
                                    "DomainValidity (网站主域名有效性检测 - 主层级域名去重 + 大站白名单豁免)"
                                },
                                "validated_copy_file_path": expanded_copy,
                                "summary": {
                                    "total_urls_checked": total_checked,
                                    "valid_or_redirected_retained_count": valid_count,
                                    "doubtful_retained_count": doubtful_count,
                                    "review_candidate_count": review_candidate_urls.len(),
                                    "onion_websites_exempt_count": exempt_count,
                                    "retained_checked_count": total_checked
                                },
                                "conclusions_and_rules": {
                                    "major_site_whitelist_rule": "根据‘常见且热门的互联网站点清单’规则（内置涵盖 YouTube, GitHub, Google, Baidu, Bing, Bilibili, Zhihu, Taobao, Twitter, Apple, Microsoft 等近 50 个核心平台，并支持通过外部 JSON 文件如 major_sites.json 进行热部署配置与扩展），系统自动豁免此类绝不可发生全站 404/倒闭的站点，直接赋予白名单跳过并保留，大幅缩减检测耗时且杜绝平台反爬防封锁。",
                                    "domain_level_validation_rule": "系统采用‘主层级域名去重校验’机制。例如 youtube.com 下所有视频仅检测主域是否存活，消减 90%+ 的重复网络请求并彻底解决 WAF/429 限流问题，凡主域名可正常访问的，下属所有书签均视为有效保留。",
                                    "content_validity_rule": "有关于内容有效性检测（--validate-content）：本功能为单独的深度内容有效性排查，检测前提是该网站主域名有效。系统对完整书签 URL 路径（例如具体的视频或贴文 URL）进行校验，当页面返回明确 HTTP 404 Not Found 或 410 Gone 时列为失效候选；反爬限流等情况继续存疑保留。",
                                    "review_candidate_rule": "单轮检查只能报告失效候选，不能作为破坏性删除依据。DNS无法解析、服务器主动拒绝连接或明确返回 404/410 的书签会列入候选清单，但校验副本保持原样，等待多轮历史确认与人工复核。",
                                    "retained_links_rule": "返回 HTTP 2xx (正常访问) 和 3xx (重定向) 的有效网址予以完全保留。",
                                    "doubtful_retained_rule": "基于死链判定五维度规则，对以下情况绝不误删并予以保留：【维度4】网络/网络代理原因导致的临时IO报错（连接超时/TLS握手错误/网络连接重置等 - LocalNetworkError/Timeout），判定为本地网络侧原因；【维度5】网站暂时维护但未来有望开放，以及开启反爬/WAF防护（HTTP 401, 403, 429, 502/503/504 ServiceUnavailable/ServerError 等）。以上全部列入‘存疑但保留’清单；同时针对 .onion 暗网地址亦免除检查予以保留。"
                                },
                                "review_candidate_items": review_candidate_items,
                                "doubtful_retained_items": doubtful_retained_items,
                                "validation_copy_preserved": true,
                                "status": "Completed (report only)"
                            });

                            info!("📊 Validity Check Summary:");
                            info!("   Total Checked: {}", total_checked);
                            info!("   Retained after review-only scan: {}", total_checked);
                            info!("   Onion Websites Exempted: {}", exempt_count);
                            info!(
                                "   Review candidates (copy unchanged): {}",
                                review_candidate_urls.len()
                            );

                            let report_path = {
                                let p = std::path::Path::new(&expanded_output);
                                let stem =
                                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
                                let dir = p.parent().unwrap_or(std::path::Path::new("."));
                                dir.join(format!("{stem}_report.json"))
                                    .to_string_lossy()
                                    .to_string()
                            };
                            if let Ok(content) = std::fs::read_to_string(&report_path) {
                                if let Ok(mut json_val) =
                                    serde_json::from_str::<serde_json::Value>(&content)
                                {
                                    json_val["validity_check_conclusions"] = validity_conclusions;
                                    if let Ok(updated_json) =
                                        serde_json::to_string_pretty(&json_val)
                                    {
                                        if let Err(e) = browser_utils::atomic_write_file(
                                            std::path::Path::new(&report_path),
                                            updated_json.as_bytes(),
                                        ) {
                                            error!("❌ 更新报告 JSON 失败: {}", e);
                                        } else {
                                            info!(
                                                "📋 已将详细的有效性检测结论与清单更新至报告: {}",
                                                report_path
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

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
            browsers: _,
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
            if rules.is_none() || classifier != "rules" {
                println!(
                    "Template organization is disabled unless --rules is provided with --classifier rules."
                );
                return Ok(());
            }
            let Some(input_files) = file else {
                println!(
                    "Browser organization is disabled; pass exported bookmark files with --file."
                );
                return Ok(());
            };

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
            sync::smart_organize_files(
                &input_files.iter().map(String::as_str).collect::<Vec<_>>(),
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
        }

        Commands::Validate { detailed } => {
            info!("🔍 Validating bookmarks...");
            let engine = SyncEngine::new()?;
            let report = engine.validate(detailed)?;
            println!("{report}");
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

        Commands::Health { command } => {
            command.execute().await.map_err(|e| anyhow::anyhow!(e))?;
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
            slow,
            history_db,
            secondary,
            scope,
            trusted_site_skip_list,
            no_trusted_site_skip,
            dns_policy,
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

            if delete {
                println!(
                    "--delete is disabled. Use `bsync health quarantine` for review, then the approved `bsync health apply-quarantine` workflow."
                );
                return Ok(());
            }
            let mut status_guard =
                CheckStatusGuard::start(export_dir.as_deref(), scope, file.as_deref());
            info!("🔍 检查收藏夹URL有效性");
            info!(
                "健康检查阶段: {}（站点与内容是独立阶段；Check 不删除）",
                match scope {
                    ProbeScope::Site => "site",
                    ProbeScope::Content => "content",
                }
            );
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            if from_file {
                if let Some(ref file_path) = file {
                    info!("输入文件: {file_path}");
                }
                if let Some(ref out) = output {
                    info!("输出文件: {out}");
                }
            }
            if let Some(ref p) = proxy {
                info!("代理: {}", p);
            } else {
                info!("代理: 未配置 (仅直连模式)");
            }
            info!("超时: {}秒", timeout);

            // Across-host parallelism stays bounded by the health engine's global and per-host
            // scheduler limits.
            let safe_concurrency = bounded_health_concurrency(concurrency);
            if concurrency > MAX_HEALTH_CONCURRENCY {
                warn!(
                    "⚠️  并发数已限制为 {}（原请求: {}）",
                    MAX_HEALTH_CONCURRENCY, concurrency
                );
            }
            info!("并发: {}", safe_concurrency);

            // 默认完整检查；需要快速抽样时由用户显式传入 --limit。
            if limit == 0 {
                info!("📚 完整检查模式：扫描全部唯一 URL");
            }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            // 读取收藏夹 - 支持从文件或浏览器读取
            let mut all_bookmarks: Vec<(browsers::BrowserType, Vec<browsers::Bookmark>)> =
                Vec::new();
            let mut target_inputs = Vec::new();
            let mut file_bookmarks: Option<Vec<browsers::Bookmark>> = None;

            if let Some(ref input_file) = file {
                // 从文件读取
                let expanded = expand_path(input_file);
                info!("📖 从文件读取: {}", expanded);
                let bookmarks = sync::import_bookmarks_from_html(&expanded)
                    .with_context(|| format!("读取收藏夹文件失败: {expanded}"))?;
                let urls = collect_urls_from_bookmarks(&bookmarks);
                let bookmark_count: usize = bookmarks.iter().map(count_tree).sum();
                info!("   {} 个书签, {} 个URL", bookmark_count, urls.len());
                target_inputs.extend(collect_bookmark_target_inputs(&bookmarks, "file"));
                file_bookmarks = Some(bookmarks);
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

                for adapter in browsers::get_all_adapters() {
                    let name = adapter.browser_type().name();
                    if browsers == "all" || browser_list.iter().any(|b| matches_browser(name, b)) {
                        match adapter.read_all_bookmarks() {
                            Ok(bookmarks) => {
                                let urls = collect_urls_from_bookmarks(&bookmarks);
                                info!(
                                    "📖 {} : {} 个收藏夹",
                                    adapter.browser_type().name(),
                                    urls.len()
                                );
                                target_inputs.extend(collect_bookmark_target_inputs(
                                    &bookmarks,
                                    adapter.browser_type().name(),
                                ));
                                all_bookmarks.push((adapter.browser_type(), bookmarks));
                            }
                            Err(e) => {
                                warn!("⚠️  {} 读取失败: {}", adapter.browser_type().name(), e);
                            }
                        }
                    }
                }

                // The legacy adapter list is intentionally small. In historical health mode,
                // supplement it with the comprehensive installation/profile harvester so Arc,
                // Edge, Chromium, Vivaldi, beta channels, and named profiles are not missed.
                if browsers == "all" && (history_db.is_some() || slow) {
                    let temp = tempfile::tempdir()?;
                    let (tree, inventory, _) =
                        sync::collector::BookmarkCollector::collect_all(temp.path())?;
                    let harvested_profiles: usize = inventory
                        .installations
                        .iter()
                        .map(|installation| installation.profiles.len())
                        .sum();
                    anyhow::ensure!(
                        harvested_profiles == inventory.total_profiles,
                        "全面 Profile 采集不完整：成功 {harvested_profiles}/{}，拒绝静默漏扫",
                        inventory.total_profiles
                    );
                    let (canonical, _, _, _, _) =
                        sync::dedupe::DedupeEngine::deduplicate_tree(&tree, &[]);
                    info!(
                        "📚 全面 Profile 采集: {} 个浏览器安装 / {} 个 Profile / {} 个唯一 URL",
                        inventory.total_installations,
                        inventory.total_profiles,
                        canonical.len()
                    );
                    for warning in inventory.warnings {
                        warn!("⚠️  {warning}");
                    }
                    for item in canonical {
                        target_inputs.push(BookmarkTargetInput {
                            bookmark_id: format!(
                                "{}:{}:{}",
                                item.primary.source.browser,
                                item.primary.source.profile_id,
                                item.primary.id
                            ),
                            original_url: item.exact_url,
                        });
                    }
                }
            }

            if target_inputs.is_empty() {
                info!("没有找到收藏夹URL");
                status_guard.finish();
                return Ok(());
            }

            let trusted_list = load_trusted_site_skip_list(trusted_site_skip_list.as_deref())?;
            let dns_policy_config = load_dns_policy(dns_policy.as_deref())?;
            let mut plan = build_probe_plan(&target_inputs, scope);
            let site_preflight_plan = if scope == ProbeScope::Content {
                let mut site_plan = build_probe_plan(&target_inputs, ProbeScope::Site);
                site_plan.apply_trusted_site_skip(&trusted_list, no_trusted_site_skip);
                let site_targets = probe_targets_from_plan(&site_plan)?;
                let confirmed_dead_site_ids = if site_targets.is_empty() {
                    HashSet::new()
                } else {
                    info!(
                        "🌐 先执行站点预检：{} 个站点；仅明确 ConfirmedDead 站点跳过逐 URL 内容探测",
                        site_targets.len()
                    );
                    let (_site_report, site_results) = run_health_stage(
                        site_targets.clone(),
                        ProbeScope::Site,
                        proxy.clone(),
                        timeout,
                        safe_concurrency,
                        slow,
                        history_db.as_deref(),
                        secondary_backends(secondary.as_deref()),
                        scoped_export_dir(export_dir.as_deref(), ProbeScope::Site)?,
                        dns_policy_config.clone(),
                    )
                    .await?;
                    site_results
                        .into_iter()
                        .filter_map(|(bookmark_id, _, verdict, _)| {
                            matches!(verdict, HealthVerdict::ConfirmedDead).then_some(bookmark_id)
                        })
                        .collect()
                };
                apply_content_site_gate(&mut plan, &confirmed_dead_site_ids);
                Some(site_plan)
            } else {
                plan.apply_trusted_site_skip(&trusted_list, no_trusted_site_skip);
                None
            };
            let partial_run = limit > 0;
            if limit > 0 && plan.targets.len() > limit {
                info!(
                    "📊 计划目标 {} 个，限制检查前 {} 个",
                    plan.targets.len(),
                    limit
                );
                plan.targets.truncate(limit);
            } else {
                info!("\n📊 计划目标 {} 个待检查", plan.targets.len());
            }
            write_probe_plan_artifact(
                &plan,
                export_dir.as_deref(),
                &trusted_list,
                trusted_site_skip_list.as_deref(),
                no_trusted_site_skip,
                &dns_policy_config,
                dns_policy.as_deref(),
                limit,
                site_preflight_plan.as_ref(),
            )?;
            let scoped_export_dir = scoped_export_dir(export_dir.as_deref(), scope)?;
            let planned_targets = probe_targets_from_plan(&plan)?;
            if planned_targets.is_empty() {
                info!("计划中没有可检查目标（invalid/skipped 已写入 probe-plan.json）");
                status_guard.finish();
                return Ok(());
            }

            // ── 新引擎路径: 提供 --history-db 或 --slow 时走 HealthEngine ──────────
            if scope == ProbeScope::Content || history_db.is_some() || slow {
                // Deterministic interleaving prevents thousands of URLs from one host from
                // monopolizing the scheduler window while preserving the selected URL set.
                let mut targets = planned_targets.clone();
                targets.sort_by_cached_key(|target| {
                    *blake3::hash(target.original_url.as_bytes()).as_bytes()
                });

                let sec_list: Vec<String> = secondary
                    .as_deref()
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                let db_path = history_db
                    .as_ref()
                    .map(|s| std::path::PathBuf::from(expand_path(s)));

                let engine = HealthEngine::new(HealthEngineOptions {
                    proxy_url: proxy.clone(),
                    timeout_secs: timeout,
                    concurrency: safe_concurrency,
                    slow_mode: slow,
                    allow_private_ips: false,
                    history_db_path: db_path,
                    secondary_backends: sec_list,
                    output_dir: scoped_export_dir.clone(),
                    partial_run,
                    max_attempts: 3,
                    retry_interval_secs: 5,
                    probe_scope: scope,
                    dns_policy: dns_policy_config.clone(),
                })
                .map_err(anyhow::Error::msg)?;

                let pb = ProgressBar::new(targets.len() as u64);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                        .unwrap_or_else(|_| ProgressStyle::default_bar())
                        .progress_chars("#>-"),
                );
                let pb2 = pb.clone();

                let report = engine
                    .run_probes(targets, move |cur, _total, _url| {
                        pb2.set_position(cur as u64);
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?;

                pb.finish_with_message("检查完成");

                println!("\n📊 健康检查结果 (多后端多历史分析)");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("  总计检查:      {}", report.total_checked);
                println!("  ✅ 有效:       {}", report.live_count);
                println!("  🔄 已恢复:     {}", report.recovered_count);
                println!("  🛡️  受保护:    {}", report.protected_count);
                println!("  ⚠️  临时失败:  {}", report.temp_failure_count);
                println!("  🔴 可能失效:   {}", report.probable_dead_count);
                println!("  ☠️  确认失效:  {}", report.confirmed_dead_count);
                println!("  ❓ 需人工审核: {}", report.needs_review_count);
                println!("  ⏱️  耗时:       {:.2}秒", report.duration_secs);
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

                if history_db.is_some() {
                    println!("\n💾 历史已写入 DB。后续运行:");
                    println!("   bsync health quarantine   -- 查看需审核的书签");
                    println!("   bsync health rebuild-summary  -- 重建汇总统计");
                }

                if delete {
                    ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true)
                        .map_err(anyhow::Error::msg)?;
                    let extreme_confirmed: HashSet<String> =
                        report.extreme_confirmed_dead_urls.iter().cloned().collect();
                    let mut cleaned = file_bookmarks.clone().unwrap_or_default();
                    let stats = remove_invalid_bookmarks_preserve_structure(
                        &mut cleaned,
                        &extreme_confirmed,
                        &RemoveConfig {
                            keep_empty_folders: keep_empty,
                        },
                    );

                    if dry_run {
                        println!(
                            "\n🧪 预览：将精确删除 {} 个极强确认失效书签；未写入文件。",
                            stats.bookmarks_removed
                        );
                    } else if let Some(out) = output.as_ref() {
                        let out = expand_path(out);
                        let temp = format!("{out}.tmp-{}", uuid::Uuid::new_v4());
                        sync::export_bookmarks_to_html(&cleaned, &temp)?;
                        if std::path::Path::new(&out).exists() {
                            let backup = format!("{out}.backup-{}", uuid::Uuid::new_v4());
                            std::fs::copy(&out, &backup)?;
                            std::fs::File::open(&backup)?.sync_all()?;
                            println!("  🛟 原输出已备份: {backup}");
                        }
                        std::fs::rename(&temp, &out)?;
                        println!(
                            "\n✅ 已保存清理副本: {out}（精确删除 {} 项，存活和存疑项均保留）",
                            stats.bookmarks_removed
                        );
                    }
                }
                status_guard.finish();
                return Ok(());
            }

            // ── 旧引擎路径 (无 --history-db / --slow) ─────────────────────────────
            let unique_urls: Vec<String> = planned_targets
                .into_iter()
                .map(|target| target.original_url)
                .collect();
            let checker = UrlChecker::new(CheckerConfig {
                proxy_url: proxy.clone(),
                timeout_secs: timeout,
                concurrency: safe_concurrency,
                retry_count: 1,
            })?;

            // 创建进度条
            let pb = ProgressBar::new(unique_urls.len() as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"));

            // 执行检查
            let start_time = std::time::Instant::now();
            let results = checker
                .check_batch(unique_urls, |current, _total, url| {
                    pb.set_position(current as u64);
                    if verbose {
                        pb.set_message(url.to_string());
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
                        println!("    代理: {pe}");
                    }
                    if let Some(ref de) = invalid.direct_error {
                        println!("    直连: {de}");
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

            // 文件模式: 导出保留全部书签的检查副本
            if from_file {
                if let Some(ref out_path) = output {
                    let out_expanded = expand_path(out_path);
                    println!("\n📤 导出保留全部书签的检查副本到: {out_expanded}");

                    if let Some(ref bookmarks) = file_bookmarks {
                        // 旧扫描只有单轮结果，不能据此从默认输出中隔离任何书签。
                        match sync::export_bookmarks_to_html(bookmarks, &out_expanded) {
                            Ok(()) => {
                                let count: usize = bookmarks.iter().map(count_tree).sum();
                                info!("✅ 保留并导出了 {} 个书签到 {}", count, out_expanded);
                            }
                            Err(e) => error!("❌ 导出失败: {}", e),
                        }
                    }
                } else {
                    warn!("⚠️  文件模式需要指定 --output 参数来保存检查副本");
                }
            }

            // 导出所有分类到目录
            if let Some(ref dir) = scoped_export_dir {
                let dir_path = dir.to_string_lossy().to_string();
                if let Err(e) = std::fs::create_dir_all(&dir_path) {
                    tracing::error!(
                        "无法创建导出目录 '{}': {}，后续导出文件可能会失败",
                        dir_path,
                        e
                    );
                }
                println!("\n📤 导出检查结果到: {dir_path}");

                // 根据模式选择书签源
                let source_bookmarks: Vec<&Vec<browsers::Bookmark>> = if from_file {
                    if let Some(b) = file_bookmarks.as_ref() {
                        vec![b]
                    } else {
                        tracing::warn!("指定了 --from-file 模式，但书签文件加载为空");
                        Vec::new()
                    }
                } else {
                    all_bookmarks.iter().map(|(_, b)| b).collect()
                };

                // 导出有效书签 (保持文件夹结构)
                if !valid_urls.is_empty() {
                    let mut valid_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &valid_urls);
                        valid_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/valid.html");
                    let actual_count: usize = valid_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&valid_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ✅ valid.html: {} 个有效书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ valid.html 导出失败: {}", e),
                    }
                }

                // 导出无效书签 (保持文件夹结构)
                if !invalid_urls.is_empty() {
                    let mut invalid_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &invalid_urls);
                        invalid_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/invalid.html");
                    let actual_count: usize = invalid_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&invalid_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ❌ invalid.html: {} 个无效书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ invalid.html 导出失败: {}", e),
                    }
                }

                // 导出不确定书签 (保持文件夹结构)
                if !uncertain_urls.is_empty() {
                    let mut uncertain_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &uncertain_urls);
                        uncertain_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/uncertain.html");
                    let actual_count: usize = uncertain_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&uncertain_bookmarks, &path) {
                        Ok(()) => info!(
                            "  ❓ uncertain.html: {} 个不确定书签 (保持文件夹结构)",
                            actual_count
                        ),
                        Err(e) => error!("  ❌ uncertain.html 导出失败: {}", e),
                    }
                }

                // 导出跳过书签 (保持文件夹结构)
                if !skipped_urls.is_empty() {
                    let mut skipped_bookmarks: Vec<browsers::Bookmark> = Vec::new();
                    for bookmarks in &source_bookmarks {
                        let extracted =
                            extract_by_status_preserve_structure(bookmarks, &skipped_urls);
                        skipped_bookmarks.extend(extracted);
                    }
                    let path = format!("{dir_path}/skipped.html");
                    let actual_count: usize = skipped_bookmarks.iter().map(count_tree).sum();
                    match sync::export_bookmarks_to_html(&skipped_bookmarks, &path) {
                        Ok(()) => info!(
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
                    println!("\n📤 导出无效收藏夹到: {export_path} (保持文件夹结构)");

                    let mut invalid_bookmarks: Vec<browsers::Bookmark> = Vec::new();
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
                        Ok(()) => info!(
                            "✅ 导出了 {} 个无效收藏夹到 {} (保持文件夹结构)",
                            invalid_bookmarks.len(),
                            export_path
                        ),
                        Err(e) => error!("❌ 导出失败: {}", e),
                    }
                }
            }
            status_guard.finish();
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
            let home = std::env::var("HOME")
                .map_err(|e| anyhow::anyhow!("获取 HOME 环境变量失败: {e}"))?;
            let db_base = match browser.to_lowercase().as_str() {
                "chrome" | "google chrome" => {
                    format!("{home}/Library/Application Support/Google/Chrome/Default")
                }
                "edge" | "microsoft edge" => {
                    format!("{home}/Library/Application Support/Microsoft Edge/Default")
                }
                "brave" => format!(
                    "{home}/Library/Application Support/BraveSoftware/Brave-Browser/Default"
                ),
                "arc" => format!("{home}/Library/Application Support/Arc/User Data/Default"),
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
                let login_db = format!("{db_base}/Login Data");
                let temp_db = temp_dir.join("LoginData");

                if std::path::Path::new(&login_db).exists() {
                    std::fs::copy(&login_db, &temp_db)?;

                    match data_types::extract_chromium_passwords(&temp_db, &browser) {
                        Ok(passwords) => {
                            let output_file = std::path::Path::new(&output_dir)
                                .join(format!("passwords_{browser}.{format}"));

                            match format.as_str() {
                                "json" => {
                                    data_types::password::export_to_json(&passwords, &output_file)?;
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

                    if let Err(e) = std::fs::remove_file(&temp_db) {
                        tracing::debug!("清理临时数据库拷贝 '{}' 失败: {}", temp_db.display(), e);
                    }
                } else {
                    warn!("   ⚠️ Login Data not found");
                }
            }

            if export_cookies {
                info!("🍪 Exporting cookies...");
                let cookies_db = format!("{db_base}/Cookies");
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
                                    data_types::cookie::export_to_netscape(&cookies, &output_file)?;
                                }
                                "json" => {
                                    data_types::cookie::export_to_json(&cookies, &output_file)?;
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

                    if let Err(e) = std::fs::remove_file(&temp_db) {
                        tracing::debug!("清理临时数据库拷贝 '{}' 失败: {}", temp_db.display(), e);
                    }
                } else {
                    warn!("   ⚠️ Cookies database not found");
                }
            }

            if export_downloads {
                info!("📥 Exporting download history...");
                let history_db = format!("{db_base}/History");
                let temp_db = temp_dir.join("History");

                if std::path::Path::new(&history_db).exists() {
                    std::fs::copy(&history_db, &temp_db)?;

                    match data_types::extract_chromium_downloads(&temp_db, &browser) {
                        Ok(downloads) => {
                            let output_file = std::path::Path::new(&output_dir)
                                .join(format!("downloads_{browser}.{format}"));

                            data_types::download::export_to_csv(&downloads, &output_file)?;

                            info!(
                                "   ✅ {} downloads exported to {}",
                                downloads.len(),
                                output_file.display()
                            );
                        }
                        Err(e) => warn!("   ⚠️ Failed to export downloads: {}", e),
                    }

                    if let Err(e) = std::fs::remove_file(&temp_db) {
                        tracing::debug!("清理临时数据库拷贝 '{}' 失败: {}", temp_db.display(), e);
                    }
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
            let export_file = format!("{expanded_out_dir}/01_exported.html");
            let mut extra_bookmarks = Vec::new();
            for inc_path in &include {
                let expanded_inc = expand_path(inc_path);
                match sync::import_bookmarks_from_source(&expanded_inc) {
                    Ok(imported) => {
                        info!(
                            "   📥 Included {} bookmarks from {}",
                            imported.len(),
                            expanded_inc
                        );
                        extra_bookmarks.extend(imported);
                    }
                    Err(e) => {
                        error!(
                            "❌ Pipeline: 无法从外部数据源 '{}' 导入书签: {}",
                            expanded_inc, e
                        );
                        return Err(
                            e.context(format!("Pipeline 从数据源 '{expanded_inc}' 导入书签失败"))
                        );
                    }
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
                exclude_onion: true,
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

            // Step 2: Check the exported copy without changing its structure.
            let results_dir = format!("{expanded_out_dir}/03_check_results");
            std::fs::create_dir_all(&results_dir)?;

            let checker = url_checker::UrlChecker::new(url_checker::CheckerConfig {
                proxy_url: proxy,
                timeout_secs: timeout,
                concurrency,
                retry_count: 1,
            })?;

            if let Ok(imported_org) = sync::import_bookmarks_from_html(&export_file) {
                let urls = url_checker::collect_urls_from_bookmarks(&imported_org);
                let mut unique_urls: Vec<String> = urls
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                if limit > 0 && unique_urls.len() > limit {
                    unique_urls.truncate(limit);
                }

                let results = checker.check_batch(unique_urls, |_, _, _| {}).await;

                let valid_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Valid)
                    .map(|r| r.url.clone())
                    .collect();
                let invalid_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Invalid)
                    .map(|r| r.url.clone())
                    .collect();
                let uncertain_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Uncertain)
                    .map(|r| r.url.clone())
                    .collect();
                let skipped_urls: std::collections::HashSet<String> = results
                    .iter()
                    .filter(|r| r.status == url_checker::ValidationStatus::Skipped)
                    .map(|r| r.url.clone())
                    .collect();

                let valid_bms =
                    url_checker::extract_by_status_preserve_structure(&imported_org, &valid_urls);
                let invalid_bms =
                    url_checker::extract_by_status_preserve_structure(&imported_org, &invalid_urls);
                let uncertain_bms = url_checker::extract_by_status_preserve_structure(
                    &imported_org,
                    &uncertain_urls,
                );
                let skipped_bms =
                    url_checker::extract_by_status_preserve_structure(&imported_org, &skipped_urls);

                if let Err(e) =
                    sync::export_bookmarks_to_html(&valid_bms, &format!("{results_dir}/valid.html"))
                {
                    tracing::error!("导出有效书签结果 (valid.html) 失败: {}", e);
                }
                if let Err(e) = sync::export_bookmarks_to_html(
                    &invalid_bms,
                    &format!("{results_dir}/invalid.html"),
                ) {
                    tracing::error!("导出无效书签结果 (invalid.html) 失败: {}", e);
                }
                if let Err(e) = sync::export_bookmarks_to_html(
                    &uncertain_bms,
                    &format!("{results_dir}/uncertain.html"),
                ) {
                    tracing::error!("导出不确定状态书签结果 (uncertain.html) 失败: {}", e);
                }
                if let Err(e) = sync::export_bookmarks_to_html(
                    &skipped_bms,
                    &format!("{results_dir}/skipped.html"),
                ) {
                    tracing::error!("导出跳过校验书签结果 (skipped.html) 失败: {}", e);
                }
            }

            // Step 3: Merge check results
            let final_file = format!("{expanded_out_dir}/FINAL_VALID.html");
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
                exclude_onion: true,
            };
            engine
                .export_to_html_with_extra(Some("waterfox"), &expanded_output, &config, vec![])
                .await?;
            info!("✅ Waterfox cleanup export complete: {}", expanded_output);
        }

        Commands::Gui { port } => {
            gui::start_gui(port).await?;
        }
    }

    Ok(())
}

fn count_tree(bookmark: &browsers::Bookmark) -> usize {
    let mut count = usize::from(bookmark.url.is_some());
    for child in &bookmark.children {
        count += count_tree(child);
    }
    count
}

fn collect_bookmark_target_inputs(
    bookmarks: &[browsers::Bookmark],
    source_prefix: &str,
) -> Vec<BookmarkTargetInput> {
    fn visit(
        bookmarks: &[browsers::Bookmark],
        source_prefix: &str,
        output: &mut Vec<BookmarkTargetInput>,
    ) {
        for bookmark in bookmarks {
            if bookmark.folder {
                visit(&bookmark.children, source_prefix, output);
            } else if let Some(url) = bookmark.url.as_deref() {
                let source_id = if bookmark.id.is_empty() {
                    format!("{source_prefix}:{url}")
                } else {
                    format!("{source_prefix}:{}", bookmark.id)
                };
                output.push(BookmarkTargetInput {
                    bookmark_id: source_id,
                    original_url: url.to_string(),
                });
            }
        }
    }

    let mut output = Vec::new();
    visit(bookmarks, source_prefix, &mut output);
    output
}

fn probe_targets_from_plan(plan: &ProbePlan) -> Result<Vec<ProbeTarget>> {
    plan.targets
        .iter()
        .map(|target| {
            let bookmark_id = match plan.scope {
                ProbeScope::Site => format!("site:{}", target.site_key),
                ProbeScope::Content => target
                    .source_bookmark_ids
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("content:{}", target.original_url)),
            };
            ProbeTarget::from_url(bookmark_id, &target.probe_url).map_err(anyhow::Error::msg)
        })
        .collect()
}

fn secondary_backends(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|backend| !backend.is_empty())
        .map(str::to_string)
        .collect()
}

async fn run_health_stage(
    targets: Vec<ProbeTarget>,
    scope: ProbeScope,
    proxy: Option<String>,
    timeout: u64,
    concurrency: usize,
    slow: bool,
    history_db: Option<&str>,
    secondary: Vec<String>,
    output_dir: Option<std::path::PathBuf>,
    dns_policy: DnsPolicy,
) -> Result<(
    HealthRunSummaryReport,
    Vec<(String, String, HealthVerdict, Vec<BackendObservation>)>,
)> {
    let db_path = history_db.map(|path| std::path::PathBuf::from(expand_path(path)));
    let engine = HealthEngine::new(HealthEngineOptions {
        proxy_url: proxy,
        timeout_secs: timeout,
        concurrency,
        slow_mode: slow,
        probe_scope: scope,
        dns_policy,
        allow_private_ips: false,
        history_db_path: db_path,
        secondary_backends: secondary,
        output_dir,
        partial_run: false,
        max_attempts: 3,
        retry_interval_secs: 5,
    })
    .map_err(anyhow::Error::msg)?;
    engine
        .run_probes_with_results(targets, |_current, _total, _url| {})
        .await
        .map_err(anyhow::Error::msg)
}

fn content_site_key(target: &browser_bookmark_sync::health::SiteTarget) -> String {
    format!(
        "{}://{}:{}",
        target.scheme, target.registrable_domain, target.effective_port
    )
}

fn apply_content_site_gate(
    content_plan: &mut ProbePlan,
    confirmed_dead_site_ids: &std::collections::HashSet<String>,
) {
    let mut retained = Vec::with_capacity(content_plan.targets.len());
    for target in content_plan.targets.drain(..) {
        let site_key = content_site_key(&target);
        if confirmed_dead_site_ids.contains(&format!("site:{site_key}")) {
            content_plan.skipped.push(SkippedTarget {
                original_url: target.probe_url.clone(),
                reason: format!("SkippedConfirmedDeadSite:{site_key}"),
                site_key: Some(site_key),
                bookmark_count: target.bookmark_count,
                source_bookmark_ids: target.source_bookmark_ids,
            });
        } else {
            retained.push(target);
        }
    }
    content_plan.targets = retained;
}

fn load_trusted_site_skip_list(path: Option<&str>) -> Result<TrustedSiteSkipList> {
    if let Some(path) = path {
        return TrustedSiteSkipList::load(expand_path(path)).map_err(anyhow::Error::msg);
    }
    serde_json::from_str(DEFAULT_TRUSTED_SITE_SKIP_LIST).map_err(anyhow::Error::msg)
}

fn load_dns_policy(path: Option<&str>) -> Result<DnsPolicy> {
    path.map_or_else(
        || Ok(DnsPolicy::default()),
        |path| DnsPolicy::load(expand_path(path)).map_err(anyhow::Error::msg),
    )
}

fn scoped_export_dir(
    export_dir: Option<&str>,
    scope: ProbeScope,
) -> Result<Option<std::path::PathBuf>> {
    export_dir
        .map(|dir| {
            let path = std::path::PathBuf::from(expand_path(dir)).join(match scope {
                ProbeScope::Site => "site",
                ProbeScope::Content => "content",
            });
            std::fs::create_dir_all(&path)?;
            Ok(path)
        })
        .transpose()
}

fn write_probe_plan_artifact(
    plan: &ProbePlan,
    export_dir: Option<&str>,
    trusted_list: &TrustedSiteSkipList,
    trusted_list_path: Option<&str>,
    no_trusted_site_skip: bool,
    dns_policy: &DnsPolicy,
    dns_policy_path: Option<&str>,
    limit: usize,
    site_preflight_plan: Option<&ProbePlan>,
) -> Result<()> {
    let Some(export_dir) = export_dir else {
        return Ok(());
    };
    let export_dir = expand_path(export_dir);
    std::fs::create_dir_all(&export_dir)?;
    let artifact = serde_json::json!({
        "scope": plan.scope,
        "output_scope": match plan.scope {
            ProbeScope::Site => "site",
            ProbeScope::Content => "content",
        },
        "targets": plan.targets,
        "invalid": plan.invalid,
        "skipped": plan.skipped,
        "target_limit": limit,
        "partial": limit > 0,
        "content_requires_live_site": false,
        "content_skips_confirmed_dead_sites": plan.scope == ProbeScope::Content,
        "site_preflight": site_preflight_plan,
        "trusted_site_skip": {
            "enabled": (matches!(plan.scope, ProbeScope::Site) || site_preflight_plan.is_some())
                && !no_trusted_site_skip,
            "path": trusted_list_path,
            "version": trusted_list.version,
            "hash": trusted_list.hash(),
        },
        "dns_policy": {
            "path": dns_policy_path,
            "config": dns_policy,
        },
    });
    let bytes = serde_json::to_vec_pretty(&artifact)?;
    browser_utils::atomic_write_file(
        &std::path::Path::new(&export_dir).join("probe-plan.json"),
        &bytes,
    )?;
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::{
        apply_content_site_gate, bounded_health_concurrency, build_probe_plan, content_site_key,
        is_disabled_non_core_command, parse_probe_scope, probe_targets_from_plan,
        BookmarkTargetInput, CheckStatusGuard, Cli, Commands, ProbeScope,
    };
    use clap::Parser;
    use std::collections::HashSet;

    #[test]
    fn network_commands_reject_zero_timeout_or_concurrency() {
        for args in [
            &["bsync", "check", "--timeout", "0"][..],
            &["bsync", "check", "--concurrency", "0"][..],
            &["bsync", "pipeline", "--timeout", "0"][..],
            &["bsync", "pipeline", "--concurrency", "0"][..],
            &["bsync", "export", "--validate-concurrency", "0"][..],
        ] {
            assert!(Cli::try_parse_from(args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn sensitive_and_gui_legacy_commands_are_disabled() {
        for args in [
            &["bsync", "history"][..],
            &["bsync", "export-data"][..],
            &["bsync", "gui"][..],
            &[
                "bsync",
                "organize",
                "--file",
                "bookmarks.html",
                "--rules",
                "rules.json",
            ][..],
            &["bsync", "pipeline"][..],
            &["bsync", "waterfox-cleanup"][..],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(is_disabled_non_core_command(&cli.command));
        }
    }

    #[test]
    fn health_concurrency_is_bounded_at_128() {
        assert_eq!(bounded_health_concurrency(64), 64);
        assert_eq!(bounded_health_concurrency(128), 128);
        assert_eq!(bounded_health_concurrency(256), 128);
    }

    #[test]
    fn check_scope_parser_accepts_only_site_or_content() {
        assert_eq!(parse_probe_scope("site").unwrap(), ProbeScope::Site);
        assert_eq!(parse_probe_scope("content").unwrap(), ProbeScope::Content);
        assert!(parse_probe_scope("auto").is_err());
        assert!(Cli::try_parse_from(["bsync", "check", "--scope", "site"]).is_ok());
        assert!(Cli::try_parse_from(["bsync", "check", "--scope", "auto"]).is_err());
        let cli = Cli::try_parse_from(["bsync", "check"]).unwrap();
        let Commands::Check { scope, .. } = cli.command else {
            panic!("expected check command");
        };
        assert_eq!(scope, ProbeScope::Content);
    }

    #[test]
    fn check_status_guard_records_running_completed_and_aborted_states() {
        let temp = tempfile::tempdir().unwrap();
        let export_dir = temp.path().to_str().unwrap();
        {
            let mut guard =
                CheckStatusGuard::start(Some(export_dir), ProbeScope::Content, Some("input.html"));
            let running: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(temp.path().join("check-status.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(running["status"], "running");
            guard.finish();
            let completed: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(temp.path().join("check-status.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(completed["status"], "completed");
        }
        {
            let _guard = CheckStatusGuard::start(Some(export_dir), ProbeScope::Content, None);
        }
        let aborted: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("check-status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(aborted["status"], "aborted");
    }

    #[test]
    fn planned_target_identity_matches_scope() {
        let site_plan = build_probe_plan(
            &[BookmarkTargetInput {
                bookmark_id: "chrome:bookmark-1".to_string(),
                original_url: "https://www.example.com/article".to_string(),
            }],
            ProbeScope::Site,
        );
        let site_target = probe_targets_from_plan(&site_plan).unwrap().remove(0);
        assert_eq!(site_target.bookmark_id, "site:https://example.com:443");
        assert_eq!(site_target.original_url, "https://example.com/");

        let content_plan = build_probe_plan(
            &[BookmarkTargetInput {
                bookmark_id: "chrome:bookmark-1".to_string(),
                original_url: "https://www.example.com/article".to_string(),
            }],
            ProbeScope::Content,
        );
        let content_target = probe_targets_from_plan(&content_plan).unwrap().remove(0);
        assert_eq!(content_target.bookmark_id, "chrome:bookmark-1");
        assert_eq!(
            content_target.original_url,
            "https://www.example.com/article"
        );
    }

    #[test]
    fn content_gate_keeps_unverified_content_for_content_level_checks() {
        let inputs = [
            BookmarkTargetInput {
                bookmark_id: "trusted".to_string(),
                original_url: "https://www.example.com/article".to_string(),
            },
            BookmarkTargetInput {
                bookmark_id: "unverified".to_string(),
                original_url: "https://www.example.net/article".to_string(),
            },
        ];
        let mut plan = build_probe_plan(&inputs, ProbeScope::Content);
        apply_content_site_gate(&mut plan, &HashSet::new());

        assert_eq!(plan.targets.len(), 2);
        assert!(plan.skipped.is_empty());

        let mut confirmed_dead = HashSet::new();
        let mut second_plan = build_probe_plan(&inputs, ProbeScope::Content);
        confirmed_dead.insert(format!(
            "site:{}",
            content_site_key(&second_plan.targets[1])
        ));
        apply_content_site_gate(&mut second_plan, &confirmed_dead);
        assert_eq!(second_plan.targets.len(), 1);
        assert_eq!(second_plan.targets[0].normalized_host, "www.example.com");
        assert!(second_plan.skipped.iter().any(|item| item.original_url
            == "https://www.example.net/article"
            && item.reason.starts_with("SkippedConfirmedDeadSite:")));
    }
}
