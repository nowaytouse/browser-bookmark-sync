//! Health Probe Engine Coordinating Tiered Backends, Scheduler, Database, and Reports

use crate::health::backends::curl::CurlCliBackend;
use crate::health::backends::dnsx::DnsxCliBackend;
use crate::health::backends::ffuf::FfufCliBackend;
use crate::health::backends::httpx::HttpxCliBackend;
use crate::health::backends::lychee::LycheeCliBackend;
use crate::health::backends::native_reqwest::NativeReqwestBackend;
use crate::health::backends::nmap::NmapCliBackend;
use crate::health::backends::openssl::OpenSslCliBackend;
use crate::health::backends::ping::PingCliBackend;
use crate::health::backends::wget::WgetCliBackend;
use crate::health::backends::ProbeBackend;

use crate::health::db::HealthDatabase;
use crate::health::models::{BackendObservation, HealthVerdict, ProbeContext, ProbeTarget};
use crate::health::reporting::{HealthRunSummaryReport, ReportExporter};
use crate::health::scheduler::{HostScheduler, SchedulerConfig};
use crate::health::state_machine::{BookmarkHistoryStats, StateMachine};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;

pub struct HealthEngineOptions {
    pub proxy_url: Option<String>,
    pub timeout_secs: u64,
    pub concurrency: usize,
    pub slow_mode: bool,
    pub allow_private_ips: bool,
    pub history_db_path: Option<PathBuf>,
    pub secondary_backends: Vec<String>,
    pub output_dir: Option<PathBuf>,
}

impl Default for HealthEngineOptions {
    fn default() -> Self {
        Self {
            proxy_url: None,
            timeout_secs: 10,
            concurrency: 5,
            slow_mode: false,
            allow_private_ips: false,
            history_db_path: None,
            secondary_backends: Vec::new(),
            output_dir: None,
        }
    }
}

pub struct HealthEngine {
    options: HealthEngineOptions,
    scheduler: HostScheduler,
    db: Option<Arc<HealthDatabase>>,
}

impl HealthEngine {
    pub fn new(options: HealthEngineOptions) -> Result<Self, String> {
        let sched_config = if options.slow_mode {
            SchedulerConfig::slow_mode()
        } else {
            SchedulerConfig {
                max_global_concurrency: options.concurrency,
                per_host_concurrency: 1,
                host_interval_ms: 200,
                default_429_cooldown_secs: 30,
            }
        };

        let scheduler = HostScheduler::new(sched_config);

        let db = if let Some(ref db_path) = options.history_db_path {
            let conn = HealthDatabase::open(db_path)
                .map_err(|e| format!("Failed to open health db: {e}"))?;
            Some(Arc::new(conn))
        } else {
            None
        };

        Ok(Self {
            options,
            scheduler,
            db,
        })
    }

    /// Execute health probe on a list of targets
    pub async fn run_probes<F>(
        &self,
        targets: Vec<ProbeTarget>,
        progress_cb: F,
    ) -> Result<HealthRunSummaryReport, String>
    where
        F: Fn(usize, usize, &str) + Send + Sync + 'static,
    {
        let run_id = uuid::Uuid::new_v4().to_string();
        let start_time = std::time::Instant::now();

        let ctx_base = ProbeContext {
            run_id: run_id.clone(),
            check_id: String::new(),
            proxy_url: self.options.proxy_url.clone(),
            timeout_secs: self.options.timeout_secs,
            slow_mode: self.options.slow_mode,
            allow_private_ips: self.options.allow_private_ips,
            history_db_path: self.options.history_db_path.clone(),
            secondary_backends: self.options.secondary_backends.clone(),
            artifact_dir: self.options.output_dir.clone(),
        };

        if let Some(ref db) = self.db {
            db.record_run_start(&ctx_base)
                .map_err(|e| format!("DB run start record failed: {e}"))?;
        }

        let total = targets.len();
        let progress_arc = Arc::new(progress_cb);

        let native_backend = NativeReqwestBackend::new();
        let lychee_backend = LycheeCliBackend::new();
        let httpx_backend = HttpxCliBackend::new();
        let curl_backend = CurlCliBackend::new();
        let dnsx_cloudflare_backend = DnsxCliBackend::cloudflare();
        let dnsx_alidns_backend = DnsxCliBackend::alidns();
        let openssl_backend = OpenSslCliBackend::new();
        let wget_backend = WgetCliBackend::new();
        let ffuf_backend = FfufCliBackend::new();
        let nmap_backend = NmapCliBackend::new();
        let ping_backend = PingCliBackend::new();

        use futures::stream::{self, StreamExt};

        let probe_results: Vec<
            Result<
                (
                    String,
                    String,
                    String,
                    HealthVerdict,
                    Vec<BackendObservation>,
                ),
                String,
            >,
        > = stream::iter(targets.into_iter().enumerate())
            .map(|(idx, target)| {
                let scheduler = self.scheduler.clone();
                let ctx_base = ctx_base.clone();
                let db_opt = self.db.clone();
                let cb = progress_arc.clone();
                let secondary_list = self.options.secondary_backends.clone();

                async move {
                    let mut key_vec = vec![target.host.as_str()];
                    if !target.registrable_domain.is_empty()
                        && target.registrable_domain != target.host
                    {
                        key_vec.push(target.registrable_domain.as_str());
                    }

                    let check_id = uuid::Uuid::new_v4().to_string();
                    let mut item_ctx = ctx_base.clone();
                    item_ctx.check_id = check_id.clone();

                    let permit = scheduler.acquire_keys(&key_vec).await;

                    cb(idx + 1, total, &target.original_url);

                    let mut observations = Vec::new();

                    // Tier 1: Native Reqwest Probe
                    if let Ok(obs) = native_backend.probe(&target, &item_ctx).await {
                        observations.push(obs);
                    }

                    let initial_verdict = StateMachine::compute_single_run_verdict(&observations);

                    // Tier 2 - Tier 5 Secondary Backends
                    // In slow mode or when initial_verdict is Live or Protected (429/403), skip dense secondary backends
                    let skip_secondary = item_ctx.slow_mode
                        && initial_verdict == HealthVerdict::Live
                        || initial_verdict == HealthVerdict::Live
                        || initial_verdict == HealthVerdict::Protected;

                    if !skip_secondary {
                        for sec in &secondary_list {
                            match sec.as_str() {
                                "lychee" => {
                                    if let Ok(obs) = lychee_backend.probe(&target, &item_ctx).await
                                    {
                                        observations.push(obs);
                                    }
                                }
                                "httpx" => {
                                    if let Ok(obs) = httpx_backend.probe(&target, &item_ctx).await {
                                        observations.push(obs);
                                    }
                                }
                                "curl" => {
                                    if let Ok(obs) = curl_backend.probe(&target, &item_ctx).await {
                                        observations.push(obs);
                                    }
                                }
                                "dnsx" => {
                                    if let Ok(obs) =
                                        dnsx_cloudflare_backend.probe(&target, &item_ctx).await
                                    {
                                        observations.push(obs);
                                    }
                                    if let Ok(obs) =
                                        dnsx_alidns_backend.probe(&target, &item_ctx).await
                                    {
                                        observations.push(obs);
                                    }
                                }
                                "openssl" => {
                                    if let Ok(obs) = openssl_backend.probe(&target, &item_ctx).await
                                    {
                                        observations.push(obs);
                                    }
                                }
                                "wget" => {
                                    if let Ok(obs) = wget_backend.probe(&target, &item_ctx).await {
                                        observations.push(obs);
                                    }
                                }
                                "ffuf" => {
                                    if let Ok(obs) = ffuf_backend.probe(&target, &item_ctx).await {
                                        observations.push(obs);
                                    }
                                }
                                "nmap" => {
                                    if let Ok(obs) = nmap_backend.probe(&target, &item_ctx).await {
                                        observations.push(obs);
                                    }
                                }
                                "ping" => {
                                    if let Ok(obs) = ping_backend.probe(&target, &item_ctx).await {
                                        observations.push(obs);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    let single_run_verdict =
                        StateMachine::compute_single_run_verdict(&observations);

                    let persistence_result = if let Some(ref db) = db_opt {
                        db.record_check_observations(
                            &item_ctx.run_id,
                            &check_id,
                            &target.bookmark_id,
                            &target.original_url,
                            single_run_verdict,
                            &observations,
                        )
                        .map_err(|error| format!("DB check record failed: {error}"))
                    } else {
                        Ok(())
                    };

                    // Register 429 Retry-After cooldown on scheduler if 429 encountered
                    let cooldown = observations.iter().find_map(|o| {
                        if o.status_code == Some(429) {
                            if let Some(ref err) = o.error_kind {
                                if err.starts_with("RetryAfter:") {
                                    let val = err.trim_start_matches("RetryAfter:");
                                    return Some(HostScheduler::parse_retry_after(val, 30));
                                }
                            }
                            Some(std::time::Duration::from_secs(30))
                        } else {
                            None
                        }
                    });

                    scheduler.release_keys(&key_vec, cooldown).await;
                    drop(permit);

                    persistence_result?;
                    Ok((
                        target.bookmark_id,
                        target.original_url,
                        check_id,
                        single_run_verdict,
                        observations,
                    ))
                }
            })
            .buffer_unordered(self.options.concurrency)
            .collect()
            .await;

        let probe_results: Vec<_> = probe_results.into_iter().collect::<Result<_, _>>()?;

        if let Some(ref db) = self.db {
            db.record_run_finish(&run_id)
                .map_err(|error| format!("DB run finish record failed: {error}"))?;
        }

        let results = if let Some(ref db) = self.db {
            let config_hash = crate::health::models::ConfigHashInput::from_probe_context(&ctx_base)
                .stable_hash()
                .map_err(|error| format!("Config hash failed: {error}"))?;
            let mut finalized = Vec::with_capacity(probe_results.len());

            for (bookmark_id, url, check_id, _single_run_verdict, observations) in probe_results {
                let evidences = db
                    .get_qualifying_run_evidences(
                        &bookmark_id,
                        Some(&config_hash),
                        crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION,
                    )
                    .map_err(|error| format!("DB evidence query failed: {error}"))?;
                let previous_verdict = db
                    .get_bookmark_current_verdict(&bookmark_id)
                    .map_err(|error| format!("DB verdict query failed: {error}"))?;
                let consecutive_successes = db
                    .get_consecutive_real_success_runs(
                        &bookmark_id,
                        Some(&config_hash),
                        crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION,
                    )
                    .map_err(|error| format!("DB success history query failed: {error}"))?;
                let summary = db
                    .get_bookmark_summary(&bookmark_id)
                    .map_err(|error| format!("DB summary query failed: {error}"))?;
                let parse_dt = |value: Option<String>| {
                    value.and_then(|value| {
                        DateTime::parse_from_rfc3339(&value)
                            .ok()
                            .map(|date| date.with_timezone(&Utc))
                    })
                };
                let history = summary.map_or_else(
                    || BookmarkHistoryStats {
                        previous_verdict,
                        consecutive_successes,
                        rule_version: crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION
                            .to_string(),
                        config_hash: config_hash.clone(),
                        qualifying_evidences: evidences.clone(),
                        ..Default::default()
                    },
                    |(
                        total_runs,
                        consecutive_failures,
                        first_failed,
                        last_failed,
                        last_success,
                    )| {
                        BookmarkHistoryStats {
                            total_runs,
                            consecutive_failures,
                            first_failed_at: parse_dt(first_failed),
                            last_failed_at: parse_dt(last_failed),
                            last_success_at: parse_dt(last_success),
                            previous_verdict,
                            consecutive_successes,
                            rule_version: crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION
                                .to_string(),
                            config_hash: config_hash.clone(),
                            qualifying_evidences: evidences.clone(),
                        }
                    },
                );
                let verdict =
                    StateMachine::resolve_final_verdict(&observations, &history, Utc::now());
                db.finalize_check_result(&run_id, &check_id, &bookmark_id, &url, verdict)
                    .map_err(|error| format!("DB verdict finalize failed: {error}"))?;
                finalized.push((bookmark_id, url, verdict, observations));
            }
            finalized
        } else {
            probe_results
                .into_iter()
                .map(
                    |(bookmark_id, url, _check_id, _single_run_verdict, observations)| {
                        let verdict = StateMachine::resolve_final_verdict(
                            &observations,
                            &BookmarkHistoryStats::default(),
                            Utc::now(),
                        );
                        (bookmark_id, url, verdict, observations)
                    },
                )
                .collect()
        };

        let duration_secs = start_time.elapsed().as_secs_f64();
        let out_dir = self
            .options
            .output_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("./bookmark-health-run"));

        let report = ReportExporter::export_all(out_dir, &run_id, duration_secs, &results)
            .map_err(|e| format!("Report generation error: {e}"))?;

        Ok(report)
    }
}
