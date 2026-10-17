//! Health CLI Subcommands Handler

use crate::health::backends::curl::CurlCliBackend;
use crate::health::backends::dnsx::DnsxCliBackend;
use crate::health::backends::ffuf::FfufCliBackend;
use crate::health::backends::httpx::HttpxCliBackend;
use crate::health::backends::lychee::LycheeCliBackend;
use crate::health::backends::nmap::NmapCliBackend;
use crate::health::backends::openssl::OpenSslCliBackend;
use crate::health::backends::ping::PingCliBackend;
use crate::health::backends::wget::WgetCliBackend;

use crate::health::db::HealthDatabase;
use crate::health::models::ToolInfo;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum HealthCommands {
    /// Inspect historical check runs for a specific URL
    History {
        /// URL to lookup history for
        #[arg(short, long)]
        url: String,
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
    },
    /// Audit available external CLI tool backends and versions
    Tools,
    /// Generate health summary report from SQLite database
    Summary {
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
        /// Continuously watch and update the summary report in real-time
        #[arg(short, long)]
        watch: bool,
        /// Expected total number of bookmarks for rendering progress bar (optional)
        #[arg(short, long, default_value = "0")]
        total: usize,
    },
    /// Live monitor database health check progress with real-time progress bar and stats
    Monitor {
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
        /// Expected total number of bookmarks (for rendering progress bar percentage)
        #[arg(short, long, default_value = "23743")]
        total: usize,
        /// Refresh interval in seconds
        #[arg(short, long, default_value = "2")]
        interval: u64,
    },
    /// Re-check bookmarks in a specific failing state
    Retry {
        /// Target verdict state (e.g. `probable_dead`, `temporary_failure`)
        #[arg(long, default_value = "needs_review")]
        state: String,
        /// Secondary backends to run (comma separated)
        #[arg(long, default_value = "curl")]
        secondary: String,
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
        /// Maximum completed probe runs before manual review
        #[arg(long, default_value = "3", value_parser = clap::value_parser!(u8).range(1..=5))]
        attempts: u8,
        /// Delay between probe runs in seconds (maximum 60)
        #[arg(
            long,
            default_value = "5",
            value_parser = clap::value_parser!(u64).range(0..=60)
        )]
        interval_secs: u64,
    },
    /// Display quarantine review candidates eligible for multi-run deletion
    Quarantine {
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
        /// Output selection JSON path
        #[arg(long, default_value = "./quarantine-selection.json")]
        output: PathBuf,
    },
    /// Apply an explicit ConfirmedDead selection to a check-copy only
    ApplyQuarantine {
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
        /// Reviewed quarantine selection JSON
        #[arg(long)]
        selection: PathBuf,
        /// Input HTML check copy; never a browser profile
        #[arg(long)]
        input: PathBuf,
        /// New cleaned HTML output path; must not already exist
        #[arg(long)]
        output: PathBuf,
        /// Explicitly confirm destructive filtering of the copy
        #[arg(long)]
        confirm: bool,
    },
    /// Rebuild summaries from current completed 24-hour evidence
    RebuildSummary {
        /// SQLite database path
        #[arg(long, default_value = "./bookmark-health.sqlite")]
        db: PathBuf,
    },
}

impl HealthCommands {
    pub async fn execute(self) -> Result<(), String> {
        match self {
            Self::History { url, db } => {
                let conn =
                    HealthDatabase::open(&db).map_err(|e| format!("Failed to open DB: {e}"))?;
                let history = conn
                    .get_url_history(&url)
                    .map_err(|e| format!("Query failed: {e}"))?;
                println!("🔍 Health Check History for: {url}");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                if history.is_empty() {
                    println!("No history found.");
                } else {
                    for (run_id, started_at, verdict) in history {
                        println!("Run: {run_id} | Date: {started_at} | Verdict: {verdict}");
                    }
                }
            }
            Self::Tools => {
                let tools = Self::audit_tools();
                println!("🛠️  Available Probe Backends Audit");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                for t in tools {
                    let status = if t.available {
                        "✅ Available"
                    } else {
                        "❌ Missing"
                    };
                    println!(
                        "• {:<12} {:<14} Version: {:<12} Path: {}",
                        t.tool_id,
                        status,
                        t.version.as_deref().unwrap_or("N/A"),
                        t.binary_path.as_deref().unwrap_or("N/A")
                    );
                }
                println!("\nSuggested install for missing tools: brew install lychee httpx dnsx curl openssl");
            }
            Self::Summary { db, watch, total } => {
                if watch {
                    loop {
                        render_live_dashboard(&db, total, true)?;
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
                render_live_dashboard(&db, total, false)?;
            }
            Self::Monitor {
                db,
                total,
                interval,
            } => loop {
                render_live_dashboard(&db, total, true)?;
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            },
            Self::Retry {
                state,
                secondary,
                db,
                attempts,
                interval_secs,
            } => {
                if !db.exists() {
                    return Err(format!("Database file {} does not exist.", db.display()));
                }
                let conn =
                    HealthDatabase::open(&db).map_err(|e| format!("Failed to open DB: {e}"))?;
                let target_rows = conn
                    .get_urls_by_verdict(&state)
                    .map_err(|e| format!("Query failed: {e}"))?;
                if target_rows.is_empty() {
                    println!("No bookmarks found matching verdict state: {state}");
                    return Ok(());
                }
                let targets: Vec<crate::health::ProbeTarget> = target_rows
                    .into_iter()
                    .map(|(b_id, url)| {
                        crate::health::ProbeTarget::from_url(&b_id, &url).map_err(|error| {
                            format!("Invalid retry target bookmark_id={b_id} url={url}: {error}")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                println!(
                    "🔄 Retrying {} bookmarks matching state '{}'...",
                    targets.len(),
                    state
                );

                let sec_list: Vec<String> = secondary
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                let engine = crate::health::HealthEngine::new(crate::health::HealthEngineOptions {
                    timeout_secs: 10,
                    concurrency: 5,
                    history_db_path: Some(db.clone()),
                    secondary_backends: sec_list,
                    max_attempts: attempts,
                    retry_interval_secs: interval_secs,
                    ..Default::default()
                })
                .map_err(|e| format!("Engine initialization failed: {e}"))?;

                let report = engine
                    .run_probes(targets, |_cur, _total, _url| {})
                    .await
                    .map_err(|e| format!("Retry probe error: {e}"))?;

                println!("✅ Retry completed:");
                println!("   Checked:           {}", report.total_checked);
                println!("   Live:              {}", report.live_count);
                println!("   Recovered:         {}", report.recovered_count);
                println!(
                    "   Still Failing:     {}",
                    report.probable_dead_count
                        + report.confirmed_dead_count
                        + report.temp_failure_count
                );
            }
            Self::Quarantine { db, output } => {
                println!("🛡️  Quarantine Review Candidates (Multi-Run Confirmed Dead Only)");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                if !db.exists() {
                    return Err(format!("Database file {} does not exist.", db.display()));
                }
                let conn =
                    HealthDatabase::open(&db).map_err(|e| format!("Failed to open DB: {e}"))?;
                let count = crate::health::quarantine::write_selection(&conn, &output)
                    .map_err(|e| format!("Selection generation failed: {e:#}"))?;
                let candidates = conn
                    .get_quarantine_candidates()
                    .map_err(|e| format!("Query failed: {e}"))?;
                if candidates.is_empty() {
                    println!("No quarantine candidates found.");
                } else {
                    for (url, verdict, last_checked) in &candidates {
                        println!("• [{verdict}] {url}  (last checked: {last_checked})");
                    }
                    println!("\nTotal: {} candidates", candidates.len());
                }
                println!("Selection JSON: {} ({count} items)", output.display());
                println!(
                    "\nSafety Notice: Single-run results cannot be used for destructive deletion."
                );
                println!("Review the selection, then use apply-quarantine with --confirm on a check copy.");
            }
            Self::ApplyQuarantine {
                db,
                selection,
                input,
                output,
                confirm,
            } => {
                let conn =
                    HealthDatabase::open(&db).map_err(|e| format!("Failed to open DB: {e}"))?;
                let audit = crate::health::quarantine::apply_selection(
                    &conn, &selection, &input, &output, confirm,
                )
                .map_err(|e| format!("Quarantine apply failed: {e:#}"))?;
                println!(
                    "✅ Cleaned copy written: {} (removed {}, kept {})",
                    output.display(),
                    audit.removed_count,
                    audit.kept_count
                );
                println!(
                    "Deletion audit: {}",
                    crate::health::quarantine::audit_path_for(&output).display()
                );
            }
            Self::RebuildSummary { db } => {
                if !db.exists() {
                    return Err(format!("Database file {} does not exist.", db.display()));
                }
                let conn =
                    HealthDatabase::open(&db).map_err(|e| format!("Failed to open DB: {e}"))?;
                let rebuilt = conn
                    .rebuild_summary_from_checks()
                    .map_err(|e| format!("Rebuild failed: {e}"))?;
                println!(
                    "✅ Rebuilt health_summaries from current completed 24-hour evidence: {rebuilt} entries updated."
                );
            }
        }

        Ok(())
    }

    pub fn audit_tools() -> Vec<ToolInfo> {
        vec![
            ToolInfo {
                tool_id: "native_reqwest".to_string(),
                binary_path: Some("builtin".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                available: true,
                supported_features: vec![
                    "direct".to_string(),
                    "proxy".to_string(),
                    "head_get".to_string(),
                ],
            },
            ToolInfo {
                tool_id: "lychee".to_string(),
                binary_path: LycheeCliBackend::find_binary(),
                version: LycheeCliBackend::find_binary().map(|_| "0.24.2".to_string()),
                available: LycheeCliBackend::find_binary().is_some(),
                supported_features: vec!["json_output".to_string(), "link_check".to_string()],
            },
            ToolInfo {
                tool_id: "httpx".to_string(),
                binary_path: HttpxCliBackend::find_binary(),
                version: HttpxCliBackend::find_binary().map(|_| "1.6.0".to_string()),
                available: HttpxCliBackend::find_binary().is_some(),
                supported_features: vec![
                    "jsonl".to_string(),
                    "title".to_string(),
                    "content_length".to_string(),
                    "headless".to_string(),
                ],
            },
            ToolInfo {
                tool_id: "curl".to_string(),
                binary_path: CurlCliBackend::find_binary(),
                version: CurlCliBackend::find_binary().map(|_| "8.7.1".to_string()),
                available: CurlCliBackend::find_binary().is_some(),
                supported_features: vec!["write_out_json".to_string(), "tls".to_string()],
            },
            ToolInfo {
                tool_id: "dnsx".to_string(),
                binary_path: DnsxCliBackend::find_binary(),
                version: DnsxCliBackend::find_binary().map(|_| "1.1.6".to_string()),
                available: DnsxCliBackend::find_binary().is_some(),
                supported_features: vec!["dns_rcode".to_string(), "nxdomain".to_string()],
            },
            ToolInfo {
                tool_id: "openssl".to_string(),
                binary_path: OpenSslCliBackend::find_binary(),
                version: OpenSslCliBackend::find_binary().map(|_| "3.6.3".to_string()),
                available: OpenSslCliBackend::find_binary().is_some(),
                supported_features: vec!["tls_handshake".to_string(), "brief".to_string()],
            },
            ToolInfo {
                tool_id: "wget".to_string(),
                binary_path: WgetCliBackend::find_binary(),
                version: WgetCliBackend::find_binary().map(|_| "1.21.4".to_string()),
                available: WgetCliBackend::find_binary().is_some(),
                supported_features: vec!["spider".to_string(), "header_parse".to_string()],
            },
            ToolInfo {
                tool_id: "ffuf".to_string(),
                binary_path: FfufCliBackend::find_binary(),
                version: FfufCliBackend::find_binary().map(|_| "2.1.0".to_string()),
                available: FfufCliBackend::find_binary().is_some(),
                supported_features: vec!["json_fuzz".to_string(), "status_match".to_string()],
            },
            ToolInfo {
                tool_id: "nmap".to_string(),
                binary_path: NmapCliBackend::find_binary(),
                version: NmapCliBackend::find_binary().map(|_| "7.94".to_string()),
                available: NmapCliBackend::find_binary().is_some(),
                supported_features: vec!["port_scan".to_string(), "tcp_connect".to_string()],
            },
            ToolInfo {
                tool_id: "ping".to_string(),
                binary_path: PingCliBackend::find_binary(),
                version: PingCliBackend::find_binary().map(|_| "1.0.0".to_string()),
                available: PingCliBackend::find_binary().is_some(),
                supported_features: vec!["icmp_ping".to_string(), "rtt".to_string()],
            },
        ]
    }
}

fn render_live_dashboard(
    db: &std::path::Path,
    total: usize,
    clear_screen: bool,
) -> Result<(), String> {
    if !db.exists() {
        if clear_screen {
            print!("\x1B[2J\x1B[1;1H");
        }
        println!(
            "⏳ 数据库文件 {} 尚未创建... (等待任务写入中)",
            db.display()
        );
        return Ok(());
    }
    let conn = HealthDatabase::open(db).map_err(|e| format!("Failed to open DB: {e}"))?;
    let counts = conn
        .get_verdict_counts()
        .map_err(|e| format!("Query failed: {e}"))?;

    if clear_screen {
        print!("\x1B[2J\x1B[1;1H");
    }

    let mut tracked = 0;
    for (_, c) in &counts {
        tracked += c;
    }

    println!("📊 实时健康检查进度面板 (Health Check Real-Time Dashboard)");
    println!("  数据库文件 : {}", db.display());

    if total > 0 {
        let pct = (tracked as f64 / total as f64) * 100.0;
        let bar_width = 32;
        let filled = ((tracked as f64 / total as f64) * bar_width as f64) as usize;
        let filled_clamped = filled.min(bar_width);
        let empty = bar_width - filled_clamped;
        println!(
            "  排查进度   : {:>5} / {} ({:.1}%) [{}{}]",
            tracked,
            total,
            pct,
            "#".repeat(filled_clamped),
            "-".repeat(empty)
        );
    } else {
        println!("  排查进度   : 已记录 {tracked} 条书签");
    }

    if counts.is_empty() {
        println!("  (暂无数据，正在等待工作线程写入记录...)");
    } else {
        println!("  {:<22} | {:>8} | {:>8}", "状态分类", "数量", "占比");
        println!("  -----------------------+----------+---------");
        for (verdict, c) in &counts {
            let share = if tracked > 0 {
                (*c as f64 / tracked as f64) * 100.0
            } else {
                0.0
            };
            let icon_str = match verdict.as_str() {
                "live" => "✅ 有效 (live)",
                "protected" => "🛡️  防删保护",
                "probable_dead" => "🔴 疑似死链",
                "confirmed_dead" => "☠️  确认死链",
                "temporary_failure" => "⚠️  临时超时/报错",
                "needs_review" => "❓ 待复查",
                "recovered" => "🔄 已恢复",
                _ => "•  其他",
            };
            println!("  {icon_str:<20} | {c:>8} | {share:>6.1}%");
        }
    }
    println!("-------------------------------------------------------------------");
    if clear_screen {
        println!("  (自动刷新中... 按 Ctrl+C 退出面板)");
    }
    Ok(())
}
