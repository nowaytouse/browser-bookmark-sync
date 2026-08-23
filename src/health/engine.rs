//! Health Probe Engine Coordinating Tiered Backends, Scheduler, Database, and Reports

use crate::health::backends::curl::CurlCliBackend;
use crate::health::backends::dnsx::DnsxCliBackend;
use crate::health::backends::ffuf::FfufCliBackend;
use crate::health::backends::httpx::HttpxCliBackend;
use crate::health::backends::lychee::LycheeCliBackend;
use crate::health::backends::native_dns::NativeDnsBackend;
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
    /// Maximum completed runs for unresolved URLs in one scan invocation.
    pub max_attempts: u8,
    /// Delay between completed retry runs.
    pub retry_interval_secs: u64,
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
            max_attempts: 3,
            retry_interval_secs: 5,
        }
    }
}

pub struct HealthEngine {
    options: HealthEngineOptions,
    scheduler: HostScheduler,
    db: Option<Arc<HealthDatabase>>,
}

const fn should_retry(verdict: HealthVerdict) -> bool {
    matches!(
        verdict,
        HealthVerdict::TemporaryFailure | HealthVerdict::ProbableDead | HealthVerdict::NeedsReview
    )
}

const MAX_RETRY_INTERVAL_SECS: u64 = 60;

const fn bounded_retry_interval_secs(interval_secs: u64) -> u64 {
    if interval_secs > MAX_RETRY_INTERVAL_SECS {
        MAX_RETRY_INTERVAL_SECS
    } else {
        interval_secs
    }
}

impl HealthEngine {
    pub fn new(options: HealthEngineOptions) -> Result<Self, String> {
        if options.concurrency == 0 {
            return Err("Health probe concurrency must be at least 1".to_string());
        }
        if options.timeout_secs == 0 {
            return Err("Health probe timeout must be at least 1 second".to_string());
        }
        if options.proxy_url.is_some() {
            if let Some(backend) = options
                .secondary_backends
                .iter()
                .find(|backend| backend.as_str() != "curl")
            {
                return Err(format!(
                    "Secondary backend '{backend}' does not support proxy routing; only curl is allowed when a proxy is configured"
                ));
            }
        }

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

    /// Execute bounded same-day probes, stopping retries as soon as a URL is live,
    /// protected, or confirmed dead.
    pub async fn run_probes<F>(
        &self,
        targets: Vec<ProbeTarget>,
        progress_cb: F,
    ) -> Result<HealthRunSummaryReport, String>
    where
        F: Fn(usize, usize, &str) + Send + Sync + 'static,
    {
        let mut target_urls_by_id = std::collections::HashMap::new();
        for target in &targets {
            if let Some(existing_url) =
                target_urls_by_id.insert(target.bookmark_id.clone(), target.original_url.clone())
            {
                if existing_url != target.original_url {
                    return Err(format!(
                        "Ambiguous bookmark ID {} maps to both {} and {}",
                        target.bookmark_id, existing_url, target.original_url
                    ));
                }
                return Err(format!(
                    "Duplicate bookmark ID {} in one probe run",
                    target.bookmark_id
                ));
            }
        }

        if let Some(ref db) = self.db {
            for target in &targets {
                if let Some(existing_url) = db
                    .find_bookmark_url_conflict(&target.bookmark_id, &target.original_url)
                    .map_err(|error| format!("DB bookmark URL identity check failed: {error}"))?
                {
                    return Err(format!(
                        "Bookmark ID {} already belongs to URL {}, not {}",
                        target.bookmark_id, existing_url, target.original_url
                    ));
                }
            }
        }

        let start_time = std::time::Instant::now();
        let scan_id = uuid::Uuid::new_v4().to_string();
        let progress = Arc::new(progress_cb);
        let max_attempts = self.options.max_attempts.max(1);
        let backend_context = ProbeContext {
            run_id: String::new(),
            check_id: String::new(),
            proxy_url: self.options.proxy_url.clone(),
            timeout_secs: self.options.timeout_secs,
            slow_mode: self.options.slow_mode,
            allow_private_ips: self.options.allow_private_ips,
            history_db_path: self.options.history_db_path.clone(),
            secondary_backends: self.options.secondary_backends.clone(),
            artifact_dir: self.options.output_dir.clone(),
        };
        let native_backend = NativeReqwestBackend::configured(&backend_context)?;
        let (dns_cloudflare_backend, dns_alidns_backend) = if self.options.proxy_url.is_none() {
            (
                Some(NativeDnsBackend::cloudflare(self.options.timeout_secs)?),
                Some(NativeDnsBackend::alidns(self.options.timeout_secs)?),
            )
        } else {
            (None, None)
        };

        let total = targets.len();
        let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        use futures::stream::{self, StreamExt};
        let final_results: Vec<_> = stream::iter(targets)
            .map(|target| {
                let progress = progress.clone();
                let completed = completed.clone();
                let native_backend = native_backend.clone();
                let dns_cloudflare_backend = dns_cloudflare_backend.clone();
                let dns_alidns_backend = dns_alidns_backend.clone();

                async move {
                    let progress_url = target.original_url.clone();
                    for attempt in 1..=max_attempts {
                        let mut round_results = self
                            .run_probe_round(
                                vec![target.clone()],
                                |_, _, _| {},
                                native_backend.clone(),
                                dns_cloudflare_backend.clone(),
                                dns_alidns_backend.clone(),
                            )
                            .await?;
                        let result = round_results.pop().ok_or_else(|| {
                            format!("Probe run returned no result for {progress_url}")
                        })?;
                        if attempt < max_attempts && should_retry(result.2) {
                            tokio::time::sleep(std::time::Duration::from_secs(
                                bounded_retry_interval_secs(self.options.retry_interval_secs),
                            ))
                            .await;
                            continue;
                        }

                        let current =
                            completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        progress(current, total, &progress_url);
                        return Ok(result);
                    }
                    Err(format!(
                        "Probe attempts exhausted without a result for {progress_url}"
                    ))
                }
            })
            .buffer_unordered(self.options.concurrency)
            .collect::<Vec<Result<_, String>>>()
            .await
            .into_iter()
            .collect::<Result<_, _>>()?;

        let duration_secs = start_time.elapsed().as_secs_f64();
        let out_dir = self
            .options
            .output_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("./bookmark-health-run"));

        ReportExporter::export_all(out_dir, &scan_id, duration_secs, &final_results)
            .map_err(|e| format!("Report generation error: {e}"))
    }

    /// Execute one independent completed probe run.
    async fn run_probe_round<F>(
        &self,
        targets: Vec<ProbeTarget>,
        progress_cb: F,
        native_backend: NativeReqwestBackend,
        dns_cloudflare_backend: Option<NativeDnsBackend>,
        dns_alidns_backend: Option<NativeDnsBackend>,
    ) -> Result<Vec<(String, String, HealthVerdict, Vec<BackendObservation>)>, String>
    where
        F: Fn(usize, usize, &str) + Send + Sync + 'static,
    {
        let run_id = uuid::Uuid::new_v4().to_string();

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
                let native_backend = native_backend.clone();
                let dns_cloudflare_backend = dns_cloudflare_backend.clone();
                let dns_alidns_backend = dns_alidns_backend.clone();

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

                    if !matches!(
                        initial_verdict,
                        HealthVerdict::Live | HealthVerdict::Protected
                    ) {
                        if let Some(backend) = dns_cloudflare_backend.as_ref() {
                            if let Ok(obs) = backend.probe(&target, &item_ctx).await {
                                observations.push(obs);
                            }
                        }
                        if let Some(backend) = dns_alidns_backend.as_ref() {
                            if let Ok(obs) = backend.probe(&target, &item_ctx).await {
                                observations.push(obs);
                            }
                        }
                    }

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

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_and_protected_results_stop_same_day_retries() {
        assert!(!should_retry(HealthVerdict::Live));
        assert!(!should_retry(HealthVerdict::Recovered));
        assert!(!should_retry(HealthVerdict::Protected));
        assert!(!should_retry(HealthVerdict::ConfirmedDead));
        assert!(should_retry(HealthVerdict::TemporaryFailure));
        assert!(should_retry(HealthVerdict::ProbableDead));
        assert!(should_retry(HealthVerdict::NeedsReview));
    }

    #[test]
    fn retry_interval_has_a_short_hard_cap() {
        assert_eq!(bounded_retry_interval_secs(0), 0);
        assert_eq!(bounded_retry_interval_secs(MAX_RETRY_INTERVAL_SECS), 60);
        assert_eq!(bounded_retry_interval_secs(86_400), 60);
    }

    #[test]
    fn engine_rejects_zero_timeout_or_concurrency() {
        let zero_concurrency = HealthEngine::new(HealthEngineOptions {
            concurrency: 0,
            ..Default::default()
        })
        .err()
        .unwrap();
        assert!(zero_concurrency.contains("concurrency"));

        let zero_timeout = HealthEngine::new(HealthEngineOptions {
            timeout_secs: 0,
            ..Default::default()
        })
        .err()
        .unwrap();
        assert!(zero_timeout.contains("timeout"));
    }

    #[test]
    fn engine_rejects_proxy_with_a_direct_secondary_backend() {
        let error = HealthEngine::new(HealthEngineOptions {
            proxy_url: Some("http://127.0.0.1:8080".to_string()),
            secondary_backends: vec!["httpx".to_string()],
            ..Default::default()
        })
        .err()
        .unwrap();

        assert!(error.contains("httpx"));
        assert!(error.contains("proxy"));

        assert!(HealthEngine::new(HealthEngineOptions {
            proxy_url: Some("http://127.0.0.1:8080".to_string()),
            secondary_backends: vec!["curl".to_string()],
            ..Default::default()
        })
        .is_ok());
    }

    #[tokio::test]
    async fn transient_retry_runs_before_an_unrelated_slow_url_finishes() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let slow_finished = Arc::new(AtomicBool::new(false));
        let retried_before_slow = Arc::new(AtomicBool::new(false));
        let fast_requests = Arc::new(AtomicUsize::new(0));
        let server_slow_finished = slow_finished.clone();
        let server_retried_before_slow = retried_before_slow.clone();
        let server_fast_requests = fast_requests.clone();
        let server = tokio::spawn(async move {
            let mut handlers = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let slow_finished = server_slow_finished.clone();
                let retried_before_slow = server_retried_before_slow.clone();
                let fast_requests = server_fast_requests.clone();
                handlers.push(tokio::spawn(async move {
                    let mut request = [0_u8; 2048];
                    let read = socket.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]);
                    if request.contains("/slow") {
                        tokio::time::sleep(std::time::Duration::from_millis(1_000)).await;
                        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = socket.write_all(response).await;
                        slow_finished.store(true, Ordering::SeqCst);
                    } else if fast_requests.fetch_add(1, Ordering::SeqCst) == 0 {
                        let response = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = socket.write_all(response).await;
                    } else {
                        retried_before_slow.store(!slow_finished.load(Ordering::SeqCst), Ordering::SeqCst);
                        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = socket.write_all(response).await;
                    }
                }));
            }
            for handler in handlers {
                handler.await.unwrap();
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health.sqlite");
        let engine = HealthEngine::new(HealthEngineOptions {
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 2,
            concurrency: 2,
            history_db_path: Some(db_path.clone()),
            output_dir: Some(temp.path().join("report")),
            max_attempts: 2,
            retry_interval_secs: 0,
            ..Default::default()
        })
        .unwrap();

        let report = engine
            .run_probes(
                vec![
                    ProbeTarget::from_url("fast", "http://93.184.216.34/fast").unwrap(),
                    ProbeTarget::from_url("slow", "http://93.184.216.35/slow").unwrap(),
                ],
                |_, _, _| {},
            )
            .await
            .unwrap();

        assert_eq!(report.total_checked, 2);
        assert_eq!(report.live_count + report.recovered_count, 2);
        assert_eq!(report.confirmed_dead_count, 0);
        server.await.unwrap();
        assert!(retried_before_slow.load(Ordering::SeqCst));
        drop(engine);

        let db = HealthDatabase::open(db_path).unwrap();
        let completed_runs: i64 = db
            .raw_conn_for_test()
            .query_row(
                "SELECT COUNT(*) FROM health_runs WHERE completed = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(completed_runs, 3);
        let direct_dns_attempts: i64 = db
            .raw_conn_for_test()
            .query_row(
                "SELECT COUNT(*) FROM health_attempts WHERE backend = 'hickory_dns'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(direct_dns_attempts, 0);
    }

    #[tokio::test]
    async fn rejects_same_run_ambiguous_bookmark_id_before_db_run_start() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health.sqlite");
        let engine = HealthEngine::new(HealthEngineOptions {
            history_db_path: Some(db_path.clone()),
            ..Default::default()
        })
        .unwrap();

        let error = engine
            .run_probes(
                vec![
                    ProbeTarget::from_url("same-id", "https://one.example/a").unwrap(),
                    ProbeTarget::from_url("same-id", "https://two.example/b").unwrap(),
                ],
                |_, _, _| {},
            )
            .await
            .unwrap_err();
        assert!(error.contains("Ambiguous bookmark ID"));
        drop(engine);

        let db = HealthDatabase::open(db_path).unwrap();
        let run_count: i64 = db
            .raw_conn_for_test()
            .query_row("SELECT COUNT(*) FROM health_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(run_count, 0);
    }

    #[tokio::test]
    async fn rejects_existing_bookmark_id_url_conflict_before_new_run() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health.sqlite");
        let db = HealthDatabase::open(&db_path).unwrap();
        let context = ProbeContext {
            run_id: "existing-run".to_string(),
            check_id: "existing-check".to_string(),
            proxy_url: None,
            timeout_secs: 1,
            slow_mode: false,
            allow_private_ips: false,
            history_db_path: Some(db_path.clone()),
            secondary_backends: Vec::new(),
            artifact_dir: None,
        };
        db.record_run_start(&context).unwrap();
        db.record_check_result(
            &context.run_id,
            &context.check_id,
            "same-id",
            "https://old.example/post",
            HealthVerdict::Live,
            &[BackendObservation::default()],
        )
        .unwrap();
        db.record_run_finish(&context.run_id).unwrap();
        drop(db);

        let engine = HealthEngine::new(HealthEngineOptions {
            history_db_path: Some(db_path.clone()),
            ..Default::default()
        })
        .unwrap();
        let error = engine
            .run_probes(
                vec![ProbeTarget::from_url("same-id", "https://new.example/post").unwrap()],
                |_, _, _| {},
            )
            .await
            .unwrap_err();
        assert!(error.contains("already belongs to URL"));
        drop(engine);

        let db = HealthDatabase::open(db_path).unwrap();
        let run_count: i64 = db
            .raw_conn_for_test()
            .query_row("SELECT COUNT(*) FROM health_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(run_count, 1);
    }
}
