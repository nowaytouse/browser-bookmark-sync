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
use crate::health::targets::{DnsPolicy, ProbeScope};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub struct HealthEngineOptions {
    pub proxy_url: Option<String>,
    pub timeout_secs: u64,
    pub concurrency: usize,
    pub slow_mode: bool,
    pub probe_scope: ProbeScope,
    pub dns_policy: DnsPolicy,
    pub allow_private_ips: bool,
    pub history_db_path: Option<PathBuf>,
    pub secondary_backends: Vec<String>,
    pub output_dir: Option<PathBuf>,
    /// Whether the caller deliberately truncated the eligible target set.
    pub partial_run: bool,
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
            probe_scope: ProbeScope::Content,
            dns_policy: DnsPolicy::default(),
            allow_private_ips: false,
            history_db_path: None,
            secondary_backends: Vec::new(),
            output_dir: None,
            partial_run: false,
            max_attempts: 3,
            retry_interval_secs: 5,
        }
    }
}

pub struct HealthEngine {
    options: HealthEngineOptions,
    scheduler: HostScheduler,
    // The SQLite connection is intentionally confined to this executor thread;
    // network probes are concurrent but database writes stay serialized.
    db: Option<Rc<HealthDatabase>>,
}

const fn should_retry(verdict: HealthVerdict) -> bool {
    matches!(
        verdict,
        HealthVerdict::TemporaryFailure
            | HealthVerdict::ProbableDead
            | HealthVerdict::ConfirmedDead
            | HealthVerdict::NeedsReview
    )
}

const fn should_skip_secondary(slow_mode: bool, verdict: HealthVerdict) -> bool {
    slow_mode || matches!(verdict, HealthVerdict::Live | HealthVerdict::Protected)
}

const MAX_RETRY_INTERVAL_SECS: u64 = 60;

const fn bounded_retry_interval_secs(interval_secs: u64) -> u64 {
    if interval_secs > MAX_RETRY_INTERVAL_SECS {
        MAX_RETRY_INTERVAL_SECS
    } else {
        interval_secs
    }
}

fn internal_probe_failure_result(
    target: &ProbeTarget,
    error: &str,
) -> (String, String, HealthVerdict, Vec<BackendObservation>) {
    let now = Utc::now();
    let observation = BackendObservation {
        backend: "health_engine".to_string(),
        backend_version: env!("CARGO_PKG_VERSION").to_string(),
        route: "internal".to_string(),
        started_at: now,
        finished_at: now,
        actual_request: "unknown".to_string(),
        request_method: "NONE".to_string(),
        initial_url: target.original_url.clone(),
        error_kind: Some(format!("InternalProbeError: {error}")),
        confidence: 0.0,
        ..Default::default()
    };
    (
        target.bookmark_id.clone(),
        target.original_url.clone(),
        HealthVerdict::NeedsReview,
        vec![observation],
    )
}

/// Finalizes a persisted run even when a probe round exits through an error path.
/// The health schema has no separate failure state, so a completed run without a
/// check is deliberately fail-closed: it cannot contribute deletion evidence.
struct RunFinishGuard {
    db: Option<Rc<HealthDatabase>>,
    run_id: String,
    finished: bool,
}

impl RunFinishGuard {
    fn new(db: Option<Rc<HealthDatabase>>, run_id: String) -> Self {
        Self {
            db,
            run_id,
            finished: false,
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        if let Some(ref db) = self.db {
            db.record_run_finish(&self.run_id)
                .map_err(|error| format!("DB run finish record failed: {error}"))?;
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for RunFinishGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(ref db) = self.db {
            if let Err(error) = db.record_run_finish(&self.run_id) {
                tracing::error!(
                    run_id = %self.run_id,
                    %error,
                    "Failed to finalize health run after probe error"
                );
            }
        }
    }
}

fn is_nxdomain(observation: &BackendObservation) -> bool {
    observation
        .dns_rcode
        .as_deref()
        .is_some_and(|rcode| rcode.eq_ignore_ascii_case("NXDOMAIN"))
}

async fn run_dns_policy<B: ProbeBackend>(
    policy: &DnsPolicy,
    target: &ProbeTarget,
    context: &ProbeContext,
    cloudflare: Option<&B>,
    alidns: Option<&B>,
) -> Vec<BackendObservation> {
    let backend = |resolver_id: &str| match resolver_id {
        "cloudflare" => cloudflare,
        "alidns" => alidns,
        _ => None,
    };
    let Some(primary_id) = policy
        .resolver_ids_for_host(&target.host)
        .into_iter()
        .next()
    else {
        return Vec::new();
    };
    let Some(primary_backend) = backend(&primary_id) else {
        return Vec::new();
    };
    let Ok(primary) = primary_backend.probe(target, context).await else {
        return Vec::new();
    };
    let needs_confirmation = is_nxdomain(&primary);
    let mut observations = vec![primary];
    if needs_confirmation {
        if let Some(confirm_backend) = policy
            .confirmation_resolvers
            .iter()
            .filter(|resolver_id| resolver_id.as_str() != primary_id)
            .find_map(|resolver_id| backend(resolver_id))
        {
            if let Ok(confirm) = confirm_backend.probe(target, context).await {
                observations.push(confirm);
            }
        }
    }
    observations
}

impl HealthEngine {
    pub fn new(options: HealthEngineOptions) -> Result<Self, String> {
        if options.concurrency == 0 {
            return Err("Health probe concurrency must be at least 1".to_string());
        }
        if options.timeout_secs == 0 {
            return Err("Health probe timeout must be at least 1 second".to_string());
        }
        options.dns_policy.validate()?;
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
            Some(Rc::new(conn))
        } else {
            None
        };

        Ok(Self {
            options,
            scheduler,
            db,
        })
    }

    fn load_same_day_positive_results(
        &self,
        targets: &[ProbeTarget],
        context: &ProbeContext,
    ) -> Result<
        std::collections::HashMap<
            (String, String),
            (String, String, HealthVerdict, Vec<BackendObservation>),
        >,
        String,
    > {
        let Some(db) = self.db.as_ref() else {
            return Ok(std::collections::HashMap::new());
        };
        let requested: std::collections::HashSet<_> = targets
            .iter()
            .map(|target| (target.bookmark_id.clone(), target.original_url.clone()))
            .collect();
        let config_hash = crate::health::models::ConfigHashInput::from_probe_context(context)
            .stable_hash()
            .map_err(|error| format!("Resume config hash failed: {error}"))?;
        let cutoff = (Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let mut statement = db
            .conn
            .prepare(
                "SELECT c.bookmark_id, c.original_url, c.run_verdict, r.run_id,
                        r.config_hash, r.rule_version,
                        COALESCE(r.finished_at, r.started_at)
                 FROM health_checks c
                 JOIN health_runs r ON r.run_id = c.run_id
                 WHERE julianday(COALESCE(r.finished_at, r.started_at)) >= julianday(?1)
                 ORDER BY julianday(COALESCE(r.finished_at, r.started_at)) DESC, c.rowid DESC",
            )
            .map_err(|error| format!("Resume query preparation failed: {error}"))?;
        let rows = statement
            .query_map(rusqlite::params![cutoff], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| format!("Resume query failed: {error}"))?;

        let mut latest_seen = std::collections::HashSet::new();
        let mut results = std::collections::HashMap::new();
        for row in rows {
            let (bookmark_id, url, verdict_text, run_id, row_config_hash, rule_version, checked_at) =
                row.map_err(|error| format!("Resume row decode failed: {error}"))?;
            let key = (bookmark_id.clone(), url.clone());
            if !requested.contains(&key) || !latest_seen.insert(key.clone()) {
                continue;
            }
            if row_config_hash.as_deref() != Some(config_hash.as_str())
                || rule_version.as_deref()
                    != Some(crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION)
            {
                continue;
            }
            let Some(verdict) = HealthVerdict::from_db_str(&verdict_text) else {
                continue;
            };
            if should_retry(verdict) {
                continue;
            }

            // A positive result is safe to reuse even when the enclosing run was interrupted:
            // it can only preserve a bookmark, never promote one into the deletion set.

            let checked_at = DateTime::parse_from_rfc3339(&checked_at)
                .map_or_else(|_| Utc::now(), |date| date.with_timezone(&Utc));
            let observation = BackendObservation {
                backend: "same_day_resume".to_string(),
                backend_version: env!("CARGO_PKG_VERSION").to_string(),
                run_id,
                route: "history".to_string(),
                started_at: checked_at,
                finished_at: checked_at,
                actual_request: "no".to_string(),
                cache_hit: true,
                request_method: "REUSE".to_string(),
                initial_url: url.clone(),
                confidence: 1.0,
                ..Default::default()
            };
            results.insert(key, (bookmark_id, url, verdict, vec![observation]));
        }
        Ok(results)
    }

    /// Execute bounded same-day probes, reusing recent positive results while retrying every
    /// failure, uncertain result, and confirmed-dead result within the current invocation.
    pub async fn run_probes<F>(
        &self,
        targets: Vec<ProbeTarget>,
        progress_cb: F,
    ) -> Result<HealthRunSummaryReport, String>
    where
        F: Fn(usize, usize, &str) + Send + Sync + 'static,
    {
        self.run_probes_with_results(targets, progress_cb)
            .await
            .map(|(report, _)| report)
    }

    /// Execute a scan and return the exact current-scan results used to build its report.
    pub async fn run_probes_with_results<F>(
        &self,
        targets: Vec<ProbeTarget>,
        progress_cb: F,
    ) -> Result<
        (
            HealthRunSummaryReport,
            Vec<(String, String, HealthVerdict, Vec<BackendObservation>)>,
        ),
        String,
    >
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
            probe_scope: self.options.probe_scope,
            dns_policy_hash: self.options.dns_policy.hash()?,
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
        let mut resumable = match self.load_same_day_positive_results(&targets, &backend_context) {
            Ok(results) => results,
            Err(error) => {
                tracing::warn!("{error}; probing all targets instead");
                std::collections::HashMap::new()
            }
        };
        let mut final_results = Vec::with_capacity(total);
        let mut targets_to_probe = Vec::with_capacity(total);
        for target in targets {
            let key = (target.bookmark_id.clone(), target.original_url.clone());
            if let Some(result) = resumable.remove(&key) {
                let current = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress(current, total, &target.original_url);
                final_results.push(result);
            } else {
                targets_to_probe.push(target);
            }
        }
        if !final_results.is_empty() {
            tracing::info!(
                "Reused {} same-day positive results; probing {} remaining targets",
                final_results.len(),
                targets_to_probe.len()
            );
        }
        // Keep enough hosts ready to avoid same-host queues starving global concurrency;
        // HostScheduler still enforces the actual global and per-host request limits.
        let scheduling_window = self.options.concurrency.saturating_mul(8);
        use futures::stream::{self, StreamExt};
        let probed_results: Vec<_> = stream::iter(targets_to_probe)
            .map(|target| {
                let progress = progress.clone();
                let completed = completed.clone();
                let native_backend = native_backend.clone();
                let dns_cloudflare_backend = dns_cloudflare_backend.clone();
                let dns_alidns_backend = dns_alidns_backend.clone();

                async move {
                    let progress_url = target.original_url.clone();
                    let probe_result = async {
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

                            return Ok(result);
                        }

                        Err(format!(
                            "Probe attempts exhausted without a result for {progress_url}"
                        ))
                    }
                    .await;

                    let result = probe_result
                        .unwrap_or_else(|error| internal_probe_failure_result(&target, &error));
                    let current = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    progress(current, total, &progress_url);
                    Ok(result)
                }
            })
            .buffer_unordered(scheduling_window)
            .collect::<Vec<Result<_, String>>>()
            .await
            .into_iter()
            .collect::<Result<_, _>>()?;
        final_results.extend(probed_results);

        let duration_secs = start_time.elapsed().as_secs_f64();
        let out_dir = self
            .options
            .output_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("./bookmark-health-run"));

        let report = ReportExporter::export_all_for_scope(
            out_dir,
            &scan_id,
            duration_secs,
            self.options.probe_scope,
            self.options.partial_run,
            &final_results,
        )
        .map_err(|e| format!("Report generation error: {e}"))?;
        Ok((report, final_results))
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
            probe_scope: self.options.probe_scope,
            dns_policy_hash: self.options.dns_policy.hash()?,
            allow_private_ips: self.options.allow_private_ips,
            history_db_path: self.options.history_db_path.clone(),
            secondary_backends: self.options.secondary_backends.clone(),
            artifact_dir: self.options.output_dir.clone(),
        };

        if let Some(ref db) = self.db {
            db.record_run_start(&ctx_base)
                .map_err(|e| format!("DB run start record failed: {e}"))?;
        }
        let mut run_finish_guard = RunFinishGuard::new(self.db.clone(), run_id.clone());

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
                let dns_policy = self.options.dns_policy.clone();

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

                    let permit = scheduler
                        .acquire_keys(&key_vec)
                        .await
                        .map_err(str::to_string)?;

                    cb(idx + 1, total, &target.original_url);

                    let mut observations = Vec::new();

                    // Tier 1: Native Reqwest Probe
                    if let Ok(obs) = native_backend.probe(&target, &item_ctx).await {
                        observations.push(obs);
                    }

                    let initial_verdict = StateMachine::compute_single_run_verdict_for_scope(
                        &observations,
                        item_ctx.probe_scope,
                    );

                    // Tier 2 - Tier 5 Secondary Backends
                    // In slow mode or when initial_verdict is Live or Protected (429/403), skip dense secondary backends
                    let skip_secondary = should_skip_secondary(item_ctx.slow_mode, initial_verdict);

                    if !matches!(
                        initial_verdict,
                        HealthVerdict::Live | HealthVerdict::Protected
                    ) {
                        observations.extend(
                            run_dns_policy(
                                &dns_policy,
                                &target,
                                &item_ctx,
                                dns_cloudflare_backend.as_ref(),
                                dns_alidns_backend.as_ref(),
                            )
                            .await,
                        );
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
                                    observations.extend(
                                        run_dns_policy(
                                            &dns_policy,
                                            &target,
                                            &item_ctx,
                                            Some(&dnsx_cloudflare_backend),
                                            Some(&dnsx_alidns_backend),
                                        )
                                        .await,
                                    );
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

                    let single_run_verdict = StateMachine::compute_single_run_verdict_for_scope(
                        &observations,
                        item_ctx.probe_scope,
                    );

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

        run_finish_guard.finish()?;

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
                let verdict = StateMachine::resolve_final_verdict_for_scope(
                    &observations,
                    &history,
                    Utc::now(),
                    self.options.probe_scope,
                );
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
                        let verdict = StateMachine::resolve_final_verdict_for_scope(
                            &observations,
                            &BookmarkHistoryStats::default(),
                            Utc::now(),
                            self.options.probe_scope,
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
        assert!(should_retry(HealthVerdict::ConfirmedDead));
        assert!(should_retry(HealthVerdict::TemporaryFailure));
        assert!(should_retry(HealthVerdict::ProbableDead));
        assert!(should_retry(HealthVerdict::NeedsReview));
    }

    #[test]
    fn slow_mode_skips_dense_secondary_backends() {
        assert!(should_skip_secondary(true, HealthVerdict::TemporaryFailure));
        assert!(should_skip_secondary(false, HealthVerdict::Live));
        assert!(should_skip_secondary(false, HealthVerdict::Protected));
        assert!(!should_skip_secondary(false, HealthVerdict::NeedsReview));
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
            probe_scope: ProbeScope::Content,
            dns_policy_hash: DnsPolicy::default().hash().unwrap(),
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

    #[tokio::test]
    async fn single_target_internal_failure_becomes_needs_review_without_aborting_batch() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 2048];
                let _ = socket.read(&mut request).await;
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health.sqlite");
        let db = HealthDatabase::open(&db_path).unwrap();
        db.raw_conn_for_test()
            .execute_batch(
                "CREATE TRIGGER fail_one_bookmark
                 BEFORE INSERT ON health_checks
                 WHEN NEW.bookmark_id = 'bad'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected per-target persistence failure');
                 END;",
            )
            .unwrap();
        drop(db);

        let report_dir = temp.path().join("report");
        let engine = HealthEngine::new(HealthEngineOptions {
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 2,
            concurrency: 2,
            history_db_path: Some(db_path.clone()),
            output_dir: Some(report_dir.clone()),
            max_attempts: 1,
            retry_interval_secs: 0,
            ..Default::default()
        })
        .unwrap();

        let report = engine
            .run_probes(
                vec![
                    ProbeTarget::from_url("good", "http://93.184.216.34/good").unwrap(),
                    ProbeTarget::from_url("bad", "http://93.184.216.35/bad").unwrap(),
                ],
                |_, _, _| {},
            )
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(report.total_checked, 2);
        assert_eq!(report.live_count, 1);
        assert_eq!(report.needs_review_count, 1);
        let review = std::fs::read_to_string(report_dir.join("needs-review.html")).unwrap();
        assert!(review.contains("http://93.184.216.35/bad"));
        assert!(review.contains("InternalProbeError"));
        drop(engine);

        let db = HealthDatabase::open(db_path).unwrap();
        let incomplete_runs: i64 = db
            .raw_conn_for_test()
            .query_row(
                "SELECT COUNT(*) FROM health_runs WHERE completed = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(incomplete_runs, 0);
    }

    #[tokio::test]
    async fn incomplete_same_day_live_result_is_reused_without_another_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut request_count = 0;
            loop {
                let accepted =
                    tokio::time::timeout(std::time::Duration::from_millis(500), listener.accept())
                        .await;
                let Ok(Ok((mut socket, _))) = accepted else {
                    break;
                };
                request_count += 1;
                let mut request = [0_u8; 2048];
                let _ = socket.read(&mut request).await;
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            }
            request_count
        });

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health.sqlite");
        let report_dir = temp.path().join("report");
        let context = ProbeContext {
            run_id: "interrupted-live-run".to_string(),
            check_id: "interrupted-live-check".to_string(),
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 2,
            slow_mode: false,
            probe_scope: ProbeScope::Content,
            dns_policy_hash: DnsPolicy::default().hash().unwrap(),
            allow_private_ips: false,
            history_db_path: Some(db_path.clone()),
            secondary_backends: Vec::new(),
            artifact_dir: Some(report_dir.clone()),
        };
        let mut observation = BackendObservation::default();
        observation.backend = "native_reqwest".to_string();
        observation.run_id = context.run_id.clone();
        observation.check_id = context.check_id.clone();
        observation.status_code = Some(200);
        observation.initial_url = "http://93.184.216.34/live".to_string();
        let db = HealthDatabase::open(&db_path).unwrap();
        db.record_run_start(&context).unwrap();
        db.record_check_observations(
            &context.run_id,
            &context.check_id,
            "live",
            &observation.initial_url,
            HealthVerdict::Live,
            &[observation.clone()],
        )
        .unwrap();
        drop(db); // Deliberately leave the run incomplete, as after an interrupted process.

        let engine = HealthEngine::new(HealthEngineOptions {
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 2,
            concurrency: 1,
            history_db_path: Some(db_path),
            output_dir: Some(report_dir.clone()),
            max_attempts: 1,
            retry_interval_secs: 0,
            ..Default::default()
        })
        .unwrap();
        let target = ProbeTarget::from_url("live", "http://93.184.216.34/live").unwrap();

        let resumed = engine.run_probes(vec![target], |_, _, _| {}).await.unwrap();

        assert_eq!(resumed.total_checked, 1);
        assert_eq!(resumed.live_count, 1);
        assert_eq!(server.await.unwrap(), 0);
        let observations =
            std::fs::read_to_string(report_dir.join("health-observations.csv")).unwrap();
        assert!(observations.contains("same_day_resume"));
    }

    #[tokio::test]
    async fn incomplete_same_day_negative_result_is_probed_again() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let accepted =
                tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept()).await;
            let Ok(Ok((mut socket, _))) = accepted else {
                return 0;
            };
            let mut request = [0_u8; 2048];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            1
        });

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health.sqlite");
        let report_dir = temp.path().join("report");
        let context = ProbeContext {
            run_id: "interrupted-negative-run".to_string(),
            check_id: "interrupted-negative-check".to_string(),
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 2,
            slow_mode: false,
            probe_scope: ProbeScope::Content,
            dns_policy_hash: DnsPolicy::default().hash().unwrap(),
            allow_private_ips: false,
            history_db_path: Some(db_path.clone()),
            secondary_backends: Vec::new(),
            artifact_dir: Some(report_dir.clone()),
        };
        let mut observation = BackendObservation::default();
        observation.backend = "native_reqwest".to_string();
        observation.run_id = context.run_id.clone();
        observation.check_id = context.check_id.clone();
        observation.status_code = Some(404);
        observation.initial_url = "http://93.184.216.34/missing".to_string();
        let db = HealthDatabase::open(&db_path).unwrap();
        db.record_run_start(&context).unwrap();
        db.record_check_observations(
            &context.run_id,
            &context.check_id,
            "missing",
            &observation.initial_url,
            HealthVerdict::ProbableDead,
            &[observation.clone()],
        )
        .unwrap();
        drop(db);

        let engine = HealthEngine::new(HealthEngineOptions {
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 2,
            concurrency: 1,
            history_db_path: Some(db_path),
            output_dir: Some(report_dir),
            max_attempts: 1,
            retry_interval_secs: 0,
            ..Default::default()
        })
        .unwrap();
        let target = ProbeTarget::from_url("missing", &observation.initial_url).unwrap();

        let report = engine.run_probes(vec![target], |_, _, _| {}).await.unwrap();

        assert_eq!(server.await.unwrap(), 1);
        assert_eq!(report.total_checked, 1);
        assert_eq!(report.live_count, 1);
    }
}
