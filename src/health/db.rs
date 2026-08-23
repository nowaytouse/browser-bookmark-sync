//! SQLite History Persistence for Health Probe System

use crate::health::models::{BackendObservation, HealthVerdict, ProbeContext};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use std::path::Path;
use tracing::info;

pub struct HealthDatabase {
    pub(crate) conn: Connection,
}

impl HealthDatabase {
    /// Open SQLite database connection and run schema migrations
    pub fn open<P: AsRef<Path>>(path: P) -> SqlResult<Self> {
        let conn = Connection::open(path)?;

        // Configure WAL mode and busy timeout for concurrent safety
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", "5000")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Create in-memory database (primarily for testing)
    pub fn in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "busy_timeout", "5000")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn raw_conn_for_test(&self) -> &Connection {
        &self.conn
    }

    /// Apply schema migrations
    fn migrate(&self) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "CREATE TABLE IF NOT EXISTS health_schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            )",
            [],
        )?;

        let current_version: usize = tx
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM health_schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            tx.execute(
                "CREATE TABLE health_runs (
                    run_id TEXT PRIMARY KEY,
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    completed INTEGER NOT NULL,
                    mode TEXT NOT NULL,
                    config_json TEXT,
                    config_hash TEXT,
                    rule_version TEXT,
                    app_version TEXT
                )",
                [],
            )?;

            tx.execute(
                "CREATE TABLE health_checks (
                    check_id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL,
                    bookmark_id TEXT NOT NULL,
                    original_url TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    run_verdict TEXT NOT NULL,
                    UNIQUE(run_id, bookmark_id)
                )",
                [],
            )?;

            tx.execute(
                "CREATE TABLE health_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    check_id TEXT NOT NULL,
                    backend TEXT NOT NULL,
                    backend_version TEXT NOT NULL,
                    route TEXT NOT NULL,
                    attempt_index INTEGER NOT NULL,
                    actual_request TEXT NOT NULL,
                    cache_hit INTEGER NOT NULL,
                    method TEXT NOT NULL,
                    status_code INTEGER,
                    error_kind TEXT,
                    initial_url TEXT NOT NULL,
                    final_url TEXT,
                    redirect_count INTEGER NOT NULL,
                    response_time_ms INTEGER NOT NULL,
                    content_type TEXT,
                    content_length INTEGER,
                    title TEXT,
                    canonical_url TEXT,
                    body_hash TEXT,
                    ip_addresses_json TEXT,
                    tls_summary TEXT,
                    confidence REAL NOT NULL
                )",
                [],
            )?;

            tx.execute(
                "CREATE TABLE health_evidence (
                    evidence_id TEXT PRIMARY KEY,
                    check_id TEXT NOT NULL,
                    attempt_id TEXT NOT NULL,
                    evidence_type TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    artifact_path TEXT
                )",
                [],
            )?;

            tx.execute(
                "CREATE TABLE health_summaries (
                    summary_id TEXT PRIMARY KEY,
                    bookmark_id TEXT NOT NULL,
                    url TEXT NOT NULL,
                    last_run_id TEXT NOT NULL,
                    last_checked_at TEXT NOT NULL,
                    current_verdict TEXT NOT NULL,
                    total_checks INTEGER NOT NULL,
                    consecutive_failures INTEGER NOT NULL,
                    first_failed_at TEXT,
                    last_failed_at TEXT,
                    last_success_at TEXT
                )",
                [],
            )?;

            tx.execute(
                "CREATE TABLE health_tool_versions (
                    tool_id TEXT PRIMARY KEY,
                    binary_path TEXT,
                    version TEXT,
                    last_verified_at TEXT NOT NULL
                )",
                [],
            )?;

            // Indexes for fast history query and URL summary lookup
            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_checks_bookmark ON health_checks(bookmark_id)",
                [],
            )?;
            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_checks_url ON health_checks(original_url)",
                [],
            )?;
            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_summaries_url ON health_summaries(url)",
                [],
            )?;
            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_attempts_check ON health_attempts(check_id)",
                [],
            )?;

            tx.execute(
                "INSERT INTO health_schema_version (version, applied_at) VALUES (1, ?1)",
                params![Utc::now().to_rfc3339()],
            )?;
        }

        if current_version < 2 {
            info!(
                "Migrating health database to version 2 (Adding DNS fields and summary versions)"
            );
            // Add columns using ALTER TABLE, ignore errors if columns already exist
            let _ = tx.execute(
                "ALTER TABLE health_attempts ADD COLUMN resolver_id TEXT",
                [],
            );
            let _ = tx.execute("ALTER TABLE health_attempts ADD COLUMN dns_rcode TEXT", []);

            let _ = tx.execute(
                "ALTER TABLE health_summaries ADD COLUMN rule_version TEXT",
                [],
            );
            let _ = tx.execute(
                "ALTER TABLE health_summaries ADD COLUMN config_hash TEXT",
                [],
            );

            tx.execute(
                "INSERT INTO health_schema_version (version, applied_at) VALUES (2, ?1)",
                params![Utc::now().to_rfc3339()],
            )?;
        }

        if current_version < 3 {
            info!("Migrating health database to version 3 (Adding per-attempt timestamps)");
            // Real attempt timestamps — NULL for pre-migration rows (intentional: old rows
            // must not become automatic strong evidence via temporal boundary checks).
            let _ = tx.execute("ALTER TABLE health_attempts ADD COLUMN started_at TEXT", []);
            let _ = tx.execute(
                "ALTER TABLE health_attempts ADD COLUMN finished_at TEXT",
                [],
            );

            tx.execute(
                "INSERT INTO health_schema_version (version, applied_at) VALUES (3, ?1)",
                params![Utc::now().to_rfc3339()],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Record start of a new health check run
    pub fn record_run_start(&self, ctx: &ProbeContext) -> SqlResult<()> {
        let hash_input = crate::health::models::ConfigHashInput::from_probe_context(ctx);
        let config_json = hash_input
            .canonical_json()
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let config_hash = hash_input
            .stable_hash()
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let rule_version = crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;

        self.conn.execute(
            "INSERT INTO health_runs (run_id, started_at, completed, mode, config_json, config_hash, rule_version, app_version)
             VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7)",
            params![
                ctx.run_id,
                Utc::now().to_rfc3339(),
                if ctx.slow_mode { "slow" } else { "standard" },
                config_json,
                config_hash,
                rule_version,
                env!("CARGO_PKG_VERSION"),
            ],
        )?;
        Ok(())
    }

    /// Mark health check run as completed
    pub fn record_run_finish(&self, run_id: &str) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE health_runs SET finished_at = ?1, completed = 1 WHERE run_id = ?2",
            params![Utc::now().to_rfc3339(), run_id],
        )?;
        Ok(())
    }

    /// Save a complete check and update its summary immediately.
    pub fn record_check_result(
        &self,
        run_id: &str,
        check_id: &str,
        bookmark_id: &str,
        url: &str,
        verdict: HealthVerdict,
        observations: &[BackendObservation],
    ) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;

        Self::insert_check_observations(
            &tx,
            run_id,
            check_id,
            bookmark_id,
            url,
            verdict,
            observations,
        )?;

        Self::update_summary_internal(&tx, bookmark_id, url, run_id, verdict)?;

        tx.commit()?;
        Ok(())
    }

    /// Persist raw observations without publishing a historical verdict.
    /// The engine uses this while the enclosing run is still incomplete.
    pub fn record_check_observations(
        &self,
        run_id: &str,
        check_id: &str,
        bookmark_id: &str,
        url: &str,
        single_run_verdict: HealthVerdict,
        observations: &[BackendObservation],
    ) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        Self::insert_check_observations(
            &tx,
            run_id,
            check_id,
            bookmark_id,
            url,
            single_run_verdict,
            observations,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Publish the final verdict only after the enclosing run is complete.
    pub fn finalize_check_result(
        &self,
        run_id: &str,
        check_id: &str,
        bookmark_id: &str,
        url: &str,
        verdict: HealthVerdict,
    ) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE health_checks SET run_verdict = ?1 WHERE check_id = ?2 AND run_id = ?3",
            params![verdict.as_str(), check_id, run_id],
        )?;
        Self::update_summary_internal(&tx, bookmark_id, url, run_id, verdict)?;
        tx.commit()?;
        Ok(())
    }

    fn insert_check_observations(
        tx: &rusqlite::Transaction<'_>,
        run_id: &str,
        check_id: &str,
        bookmark_id: &str,
        url: &str,
        verdict: HealthVerdict,
        observations: &[BackendObservation],
    ) -> SqlResult<()> {
        tx.execute(
            "INSERT OR REPLACE INTO health_checks (check_id, run_id, bookmark_id, original_url, started_at, finished_at, run_verdict)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                check_id,
                run_id,
                bookmark_id,
                url,
                Utc::now().to_rfc3339(),
                Utc::now().to_rfc3339(),
                verdict.as_str(),
            ],
        )?;

        for (idx, obs) in observations.iter().enumerate() {
            let attempt_id = uuid::Uuid::new_v4().to_string();
            let ips_json = serde_json::to_string(&obs.ip_addresses).unwrap_or_default();

            tx.execute(
                "INSERT INTO health_attempts (
                    attempt_id, check_id, backend, backend_version, route, attempt_index,
                    actual_request, cache_hit, method, status_code, error_kind, initial_url,
                    final_url, redirect_count, response_time_ms, content_type, content_length,
                    title, canonical_url, body_hash, ip_addresses_json, tls_summary, confidence,
                    resolver_id, dns_rcode, started_at, finished_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
                params![
                    attempt_id,
                    check_id,
                    obs.backend,
                    obs.backend_version,
                    obs.route,
                    idx as i64,
                    obs.actual_request,
                    i32::from(obs.cache_hit),
                    obs.request_method,
                    obs.status_code,
                    obs.error_kind,
                    obs.initial_url,
                    obs.final_url,
                    obs.redirect_count as i64,
                    obs.response_time_ms as i64,
                    obs.content_type,
                    obs.content_length.map(|l| l as i64),
                    obs.title,
                    obs.canonical_url,
                    obs.body_hash,
                    ips_json,
                    obs.tls_summary,
                    obs.confidence,
                    obs.resolver_id,
                    obs.dns_rcode,
                    obs.started_at.to_rfc3339(),
                    obs.finished_at.to_rfc3339(),
                ],
            )?;
        }

        Ok(())
    }

    fn update_summary_internal(
        tx: &rusqlite::Transaction<'_>,
        bookmark_id: &str,
        url: &str,
        run_id: &str,
        verdict: HealthVerdict,
    ) -> SqlResult<()> {
        let now_str = Utc::now().to_rfc3339();

        let existing: Option<(String, String, usize, usize, Option<String>, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT summary_id, last_run_id, total_checks, consecutive_failures, first_failed_at, last_failed_at, last_success_at
                 FROM health_summaries WHERE bookmark_id = ?1",
                params![bookmark_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;

        let is_failure = matches!(
            verdict,
            HealthVerdict::ProbableDead
                | HealthVerdict::ConfirmedDead
                | HealthVerdict::TemporaryFailure
        );

        let is_success = matches!(verdict, HealthVerdict::Live | HealthVerdict::Recovered);

        if let Some((id, last_r_id, total, cons_fail, first_fail, _, last_succ)) = existing {
            let is_same_run = last_r_id == run_id;
            let new_total = if is_same_run { total } else { total + 1 };
            let (new_cons_fail, new_first_fail, new_last_fail, new_last_succ) = if is_failure {
                (
                    if is_same_run {
                        cons_fail
                    } else {
                        cons_fail + 1
                    },
                    first_fail.or_else(|| Some(now_str.clone())),
                    Some(now_str.clone()),
                    last_succ,
                )
            } else if is_success {
                (0, None, None, Some(now_str.clone()))
            } else {
                (cons_fail, first_fail, None, last_succ)
            };

            tx.execute(
                "UPDATE health_summaries SET
                    url = ?1, last_run_id = ?2, last_checked_at = ?3, current_verdict = ?4,
                    total_checks = ?5, consecutive_failures = ?6, first_failed_at = ?7,
                    last_failed_at = ?8, last_success_at = ?9
                 WHERE summary_id = ?10",
                params![
                    url,
                    run_id,
                    now_str,
                    verdict.as_str(),
                    new_total,
                    new_cons_fail,
                    new_first_fail,
                    new_last_fail,
                    new_last_succ,
                    id,
                ],
            )?;
        } else {
            let summary_id = uuid::Uuid::new_v4().to_string();
            let cons_fail = i32::from(is_failure);
            let first_fail = if is_failure {
                Some(now_str.clone())
            } else {
                None
            };
            let last_fail = if is_failure {
                Some(now_str.clone())
            } else {
                None
            };
            let last_succ = if is_success {
                Some(now_str.clone())
            } else {
                None
            };

            tx.execute(
                "INSERT INTO health_summaries (
                    summary_id, bookmark_id, url, last_run_id, last_checked_at, current_verdict,
                    total_checks, consecutive_failures, first_failed_at, last_failed_at, last_success_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?10)",
                params![
                    summary_id,
                    bookmark_id,
                    url,
                    run_id,
                    now_str,
                    verdict.as_str(),
                    cons_fail,
                    first_fail,
                    last_fail,
                    last_succ,
                ],
            )?;
        }

        Ok(())
    }

    /// Query past check history for a specific URL
    pub fn get_url_history(&self, url_str: &str) -> SqlResult<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, started_at, run_verdict FROM health_checks WHERE original_url = ?1 ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![url_str], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    /// Get summary stats for a bookmark
    pub fn get_bookmark_summary(
        &self,
        bookmark_id: &str,
    ) -> SqlResult<Option<(usize, usize, Option<String>, Option<String>, Option<String>)>> {
        self.conn
            .query_row(
                "SELECT total_checks, consecutive_failures, first_failed_at, last_failed_at, last_success_at
                 FROM health_summaries WHERE bookmark_id = ?1",
                params![bookmark_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()
    }

    /// Return an existing URL for a bookmark ID when it conflicts with `url`.
    ///
    /// Health history is keyed by bookmark ID today, so callers must reject an
    /// ID being reused for a different URL before recording a new run.
    pub fn find_bookmark_url_conflict(
        &self,
        bookmark_id: &str,
        url: &str,
    ) -> SqlResult<Option<String>> {
        self.conn
            .query_row(
                "SELECT original_url
                 FROM health_checks
                 WHERE bookmark_id = ?1 AND original_url <> ?2
                 UNION
                 SELECT url
                 FROM health_summaries
                 WHERE bookmark_id = ?1 AND url <> ?2
                 LIMIT 1",
                params![bookmark_id, url],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn get_bookmark_current_verdict(
        &self,
        bookmark_id: &str,
    ) -> SqlResult<Option<HealthVerdict>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT current_verdict FROM health_summaries WHERE bookmark_id = ?1",
                params![bookmark_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.as_deref().and_then(HealthVerdict::from_db_str))
    }

    /// Build the qualifying run evidence set for a bookmark from raw attempts/checks/runs.
    ///
    /// A run qualifies only when:
    ///   - run.completed = 1
    ///   - all attempts within the run for this `bookmark_id` have `actual_request` = 'yes'
    ///     AND `cache_hit` = 0
    ///   - the run occurred AFTER the last real success for the bookmark
    ///   - the run contains NO real content-aware success (uncached real 2xx with a body hash)
    ///   - the run contains at least one allowed strong failure
    ///   - the run shares `config_hash` with `current_config_hash` (if provided)
    ///
    /// Each `run_id` contributes at most one `QualifyingRunEvidence` entry.
    pub fn get_qualifying_run_evidences(
        &self,
        bookmark_id: &str,
        current_config_hash: Option<&str>,
        current_rule_version: &str,
    ) -> SqlResult<Vec<crate::health::models::QualifyingRunEvidence>> {
        use crate::health::models::{QualifyingRunEvidence, StrongFailureKind};
        use chrono::DateTime;
        use std::collections::{BTreeMap, BTreeSet};

        // 1. Find the last real success timestamp for this bookmark.
        //    Pre-v3 rows have no attempt timestamp and cannot be promoted into evidence.
        let last_real_success: Option<String> = self
            .conn
            .query_row(
                "SELECT MAX(a.started_at)
             FROM health_attempts a
             JOIN health_checks c ON c.check_id = a.check_id
             JOIN health_runs r ON r.run_id = c.run_id
             WHERE c.bookmark_id = ?1
               AND r.completed = 1
               AND (?2 IS NULL OR r.config_hash = ?2)
               AND r.rule_version = ?3
               AND a.started_at IS NOT NULL
               AND a.actual_request = 'yes'
               AND a.cache_hit = 0
               AND a.status_code >= 200
               AND a.status_code < 300
               AND a.error_kind IS NULL
               AND (
                   (
                       a.body_hash IS NOT NULL
                       AND a.body_hash != ''
                       AND a.content_length > 0
                   )
                   OR NOT EXISTS (
                       SELECT 1
                       FROM health_attempts semantic_attempt
                       JOIN health_checks semantic_check
                         ON semantic_check.check_id = semantic_attempt.check_id
                       WHERE semantic_check.run_id = r.run_id
                         AND semantic_check.bookmark_id = c.bookmark_id
                         AND semantic_attempt.error_kind IN (
                             'ContentMissing', 'RedirectedToHomepage', 'Soft404Candidate'
                         )
                   )
               )",
                params![bookmark_id, current_config_hash, current_rule_version],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let last_success_dt: Option<DateTime<Utc>> = last_real_success.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        });

        // 2. Query all completed runs that have at least one strong-failure attempt
        //    for this bookmark, after the last success, matching config_hash if required.
        //    Only real per-attempt timestamps are accepted. Check/run timestamps are not
        //    substitutes because they can cross a later-success boundary.
        let config_filter = if current_config_hash.is_some() {
            "AND r.config_hash = ?2"
        } else {
            "AND 1=1" // no filter
        };

        let sql = format!(
            "SELECT
                r.run_id,
                r.finished_at,
                a.started_at AS attempt_time,
                a.backend,
                a.status_code,
                a.error_kind,
                a.dns_rcode,
                a.resolver_id,
                a.actual_request,
                a.cache_hit,
                a.body_hash,
                a.content_length
             FROM health_attempts a
             JOIN health_checks  c ON c.check_id = a.check_id
             JOIN health_runs    r ON r.run_id   = c.run_id
             WHERE c.bookmark_id = ?1
               AND r.completed   = 1
               AND a.started_at IS NOT NULL
               AND r.rule_version = ?3
               {config_filter}
             ORDER BY r.run_id, a.attempt_id"
        );

        let mut stmt = self.conn.prepare(&sql)?;

        struct AttemptRow {
            run_id: String,
            finished_at: Option<String>,
            attempt_time: String,
            backend: String,
            status_code: Option<i64>,
            error_kind: Option<String>,
            dns_rcode: Option<String>,
            resolver_id: Option<String>,
            actual_request: String,
            cache_hit: i64,
            body_hash: Option<String>,
            content_length: Option<i64>,
        }

        let rows: Vec<AttemptRow> = if let Some(hash) = current_config_hash {
            stmt.query_map(params![bookmark_id, hash, current_rule_version], |row| {
                Ok(AttemptRow {
                    run_id: row.get(0)?,
                    finished_at: row.get(1)?,
                    attempt_time: row.get(2)?,
                    backend: row.get(3)?,
                    status_code: row.get(4)?,
                    error_kind: row.get(5)?,
                    dns_rcode: row.get(6)?,
                    resolver_id: row.get(7)?,
                    actual_request: row.get(8)?,
                    cache_hit: row.get(9)?,
                    body_hash: row.get(10)?,
                    content_length: row.get(11)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?
        } else {
            stmt.query_map(params![bookmark_id, "", current_rule_version], |row| {
                Ok(AttemptRow {
                    run_id: row.get(0)?,
                    finished_at: row.get(1)?,
                    attempt_time: row.get(2)?,
                    backend: row.get(3)?,
                    status_code: row.get(4)?,
                    error_kind: row.get(5)?,
                    dns_rcode: row.get(6)?,
                    resolver_id: row.get(7)?,
                    actual_request: row.get(8)?,
                    cache_hit: row.get(9)?,
                    body_hash: row.get(10)?,
                    content_length: row.get(11)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?
        };

        // Group by run_id
        let mut by_run: BTreeMap<String, (Option<String>, Vec<AttemptRow>)> = BTreeMap::new();
        for row in rows {
            let entry = by_run
                .entry(row.run_id.clone())
                .or_insert_with(|| (row.finished_at.clone(), Vec::new()));
            entry.1.push(row);
        }

        fn is_real_success(a: &AttemptRow) -> bool {
            a.actual_request == "yes"
                && a.cache_hit == 0
                && a.status_code.is_some_and(|c| (200..300).contains(&c))
        }

        fn is_content_aware_success(a: &AttemptRow) -> bool {
            is_real_success(a)
                && a.error_kind.is_none()
                && a.body_hash.as_deref().is_some_and(|hash| !hash.is_empty())
                && a.content_length.is_some_and(|length| length > 0)
        }

        fn strong_failure_kind(a: &AttemptRow) -> Option<StrongFailureKind> {
            if a.actual_request != "yes" || a.cache_hit != 0 {
                return None;
            }
            if let Some(code) = a.status_code {
                match code {
                    404 => return Some(StrongFailureKind::Http404),
                    410 => return Some(StrongFailureKind::Http410),
                    _ => {}
                }
            }
            if let Some(ref err) = a.error_kind {
                if err == "RedirectedToHomepage" {
                    return Some(StrongFailureKind::RedirectedToHomepage);
                }
                if err == "ContentMissing" {
                    return Some(StrongFailureKind::ContentMissing);
                }
                if err == "DnsNxdomain" {
                    return Some(StrongFailureKind::ConfirmedNxDomain);
                }
            }
            // NxDomain via structured dns_rcode (NOT from error string)
            if let Some(ref rcode) = a.dns_rcode {
                if rcode.eq_ignore_ascii_case("NxDomain") {
                    // Only ConfirmedNxDomain if multi-resolver agreement (checked later per-run)
                    // Mark as candidate here; promotion to ConfirmedNxDomain happens below
                    return Some(StrongFailureKind::ConfirmedNxDomain);
                }
            }
            None
        }

        fn verifier_family(backend: &str) -> Option<&'static str> {
            let lower_owned = backend.to_lowercase();
            let lower = lower_owned.as_str();
            if lower.contains("reqwest")
                || lower.contains("native")
                || lower == "direct"
                || lower == "proxy"
            {
                Some("native")
            } else if lower.contains("lychee") {
                Some("lychee")
            } else if lower.contains("httpx") {
                Some("httpx")
            } else if lower.contains("curl") {
                Some("curl")
            } else if lower.contains("dns")
                || lower.contains("hickory")
                || lower.contains("resolver")
            {
                Some("dns")
            } else if lower.contains("headless")
                || lower.contains("playwright")
                || lower.contains("chromium")
            {
                Some("headless")
            } else {
                None // unknown backend — does not create a family
            }
        }

        let mut evidences: Vec<QualifyingRunEvidence> = Vec::new();

        for (run_id, (finished_at, attempts)) in by_run {
            if attempts
                .iter()
                .any(|attempt| attempt.actual_request != "yes" || attempt.cache_hit != 0)
            {
                continue;
            }
            let has_content_signal = attempts.iter().any(|attempt| {
                matches!(
                    attempt.error_kind.as_deref(),
                    Some("ContentMissing" | "RedirectedToHomepage" | "Soft404Candidate")
                )
            });
            // A 2xx transport response is expected for a soft-404. It is a real success only
            // when no content-aware probe in the same run found the deep resource missing.
            if attempts.iter().any(is_content_aware_success)
                || (!has_content_signal && attempts.iter().any(is_real_success))
            {
                continue;
            }

            // Collect strong failure kinds (excluding ConfirmedNxDomain — need resolver agreement)
            let mut sfk: BTreeSet<StrongFailureKind> = BTreeSet::new();
            let mut families: BTreeSet<String> = BTreeSet::new();
            let mut nxdomain_resolver_ids: BTreeSet<String> = BTreeSet::new();
            let mut has_non_nxdomain_dns_rcode = false;
            let mut qualifying_times: Vec<DateTime<Utc>> = Vec::new();
            let mut nxdomain_times: Vec<DateTime<Utc>> = Vec::new();

            for a in &attempts {
                if a.actual_request != "yes" || a.cache_hit != 0 {
                    continue;
                }
                let Some(attempt_at) = DateTime::parse_from_rfc3339(&a.attempt_time)
                    .ok()
                    .map(|time| time.with_timezone(&Utc))
                else {
                    continue;
                };
                if last_success_dt.is_some_and(|last_success| attempt_at <= last_success) {
                    continue;
                }
                if let Some(ref rcode) = a.dns_rcode {
                    if rcode.eq_ignore_ascii_case("NxDomain") {
                        // Only store resolver_id if it is explicitly provided
                        if let Some(ref rid) = a.resolver_id {
                            if !rid.is_empty() {
                                nxdomain_resolver_ids.insert(rid.clone());
                                nxdomain_times.push(attempt_at);
                            }
                        }
                    } else {
                        has_non_nxdomain_dns_rcode = true;
                    }
                }
                match strong_failure_kind(a) {
                    Some(StrongFailureKind::ConfirmedNxDomain) => {
                        // Handled below after resolver check
                    }
                    Some(kind) => {
                        sfk.insert(kind);
                        qualifying_times.push(attempt_at);
                        if let Some(fam) = verifier_family(&a.backend) {
                            families.insert(fam.to_string());
                        }
                    }
                    None => {}
                }
            }

            // DNS resolver agreement: require 2+ distinct resolver_ids, and NO conflicting rcodes
            if nxdomain_resolver_ids.len() >= 2 && !has_non_nxdomain_dns_rcode {
                sfk.insert(StrongFailureKind::ConfirmedNxDomain);
                families.insert("dns".to_string());
                qualifying_times.extend(nxdomain_times);
            }

            if sfk.is_empty() {
                continue; // No qualifying strong failure
            }

            let completed_at = finished_at
                .as_deref()
                .and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|d| d.with_timezone(&Utc))
                })
                .unwrap_or_else(Utc::now);

            // Use earliest real attempt timestamp as the strong_failure_at for span calculation.
            let strong_failure_at = qualifying_times.into_iter().min().unwrap_or(completed_at);

            evidences.push(QualifyingRunEvidence {
                run_id,
                completed_at,
                strong_failure_at,
                verifier_families: families,
                strong_failure_kinds: sfk,
            });
        }

        Ok(evidences)
    }

    /// Return current-day quarantine candidates whose latest verdict is `probable_dead` or
    /// `confirmed_dead`. Older verdicts are expired and must be probed again.
    /// Returns (url, `current_verdict`, `last_checked_at`) tuples ordered by verdict severity.
    pub fn get_quarantine_candidates(&self) -> SqlResult<Vec<(String, String, String)>> {
        let cutoff = (Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let mut stmt = self.conn.prepare(
            "SELECT url, current_verdict, last_checked_at
             FROM health_summaries
             WHERE current_verdict IN ('confirmed_dead', 'probable_dead')
               AND julianday(last_checked_at) >= julianday(?1)
             ORDER BY
               CASE current_verdict
                 WHEN 'confirmed_dead' THEN 0
                 WHEN 'probable_dead'  THEN 1
                 ELSE 2
               END,
               last_checked_at DESC",
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    /// Return counts of bookmarks grouped by `current_verdict` in `health_summaries`.
    pub fn get_verdict_counts(&self) -> SqlResult<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT current_verdict, COUNT(*)
             FROM health_summaries
             GROUP BY current_verdict
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let count: i64 = row.get(1)?;
            Ok((row.get(0)?, count as usize))
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    /// Return (`bookmark_id`, url) pairs for all bookmarks matching a specific verdict.
    /// If verdict is "`all_failing`" or "failing", matches `probable_dead`, `confirmed_dead`, `temporary_failure`, `needs_review`.
    pub fn get_urls_by_verdict(&self, verdict: &str) -> SqlResult<Vec<(String, String)>> {
        let sql = if verdict == "all_failing" || verdict == "failing" {
            "SELECT bookmark_id, url FROM health_summaries WHERE current_verdict IN ('probable_dead', 'confirmed_dead', 'temporary_failure', 'needs_review')"
        } else {
            "SELECT bookmark_id, url FROM health_summaries WHERE current_verdict = ?1"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let mut results = Vec::new();
        if verdict == "all_failing" || verdict == "failing" {
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            for r in rows {
                results.push(r?);
            }
        } else {
            let rows = stmt.query_map(params![verdict], |row| Ok((row.get(0)?, row.get(1)?)))?;
            for r in rows {
                results.push(r?);
            }
        }
        Ok(results)
    }

    /// Rebuild `health_summaries` from scratch using `health_checks` as authoritative source.
    ///
    /// Per plan §10: "任何 summary 必须可以通过 `health rebuild-summary` 从 attempts/checks 完整重建。"
    ///
    /// This wipes the summary table and recomputes per-bookmark aggregates from `health_checks`.
    /// Returns the number of summary rows written.
    pub fn rebuild_summary_from_checks(&self) -> SqlResult<usize> {
        let tx = self.conn.unchecked_transaction()?;

        // Wipe current summaries — they are a cache and must be fully rebuildable.
        tx.execute("DELETE FROM health_summaries", [])?;

        // Group rows by bookmark_id in application code for portability
        use std::collections::BTreeMap;
        struct BmData {
            url: String,
            rows: Vec<(String, String)>, // (verdict, finished_at)
        }
        let mut by_bm: BTreeMap<String, BmData> = BTreeMap::new();

        {
            let mut stmt = tx.prepare(
                "SELECT
                   bookmark_id,
                   original_url,
                   run_verdict,
                   finished_at
                 FROM health_checks
                 WHERE finished_at IS NOT NULL
                 ORDER BY bookmark_id, finished_at ASC",
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?, // bookmark_id
                    row.get::<_, String>(1)?, // original_url
                    row.get::<_, String>(2)?, // run_verdict
                    row.get::<_, String>(3)?, // finished_at
                ))
            })?;

            for r in rows {
                let (bm_id, url, verdict, finished_at) = r?;
                let entry = by_bm.entry(bm_id).or_insert_with(|| BmData {
                    url: url.clone(),
                    rows: Vec::new(),
                });
                entry.rows.push((verdict, finished_at));
            }
        } // stmt dropped here — tx borrow released

        let now_str = Utc::now().to_rfc3339();
        let mut written = 0usize;

        for (bm_id, data) in &by_bm {
            let total_checks = data.rows.len();
            if total_checks == 0 {
                continue;
            }

            let is_strong_failure = |v: &str| matches!(v, "probable_dead" | "confirmed_dead");
            let is_success = |v: &str| matches!(v, "live" | "recovered");

            let mut consecutive_failures: usize = 0;
            let mut first_failed_at: Option<String> = None;
            let mut last_failed_at: Option<String> = None;
            let mut last_success_at: Option<String> = None;
            let mut current_verdict = String::new();
            let mut last_checked_at = String::new();

            for (verdict, finished_at) in &data.rows {
                current_verdict = verdict.clone();
                last_checked_at = finished_at.clone();

                if is_success(verdict) {
                    consecutive_failures = 0;
                    first_failed_at = None;
                    last_success_at = Some(finished_at.clone());
                } else if is_strong_failure(verdict) {
                    consecutive_failures += 1;
                    if first_failed_at.is_none() {
                        first_failed_at = Some(finished_at.clone());
                    }
                    last_failed_at = Some(finished_at.clone());
                }
            }

            let summary_id = uuid::Uuid::new_v4().to_string();
            // Find the most recent run_id for this bookmark
            let last_run_id: String = tx
                .query_row(
                    "SELECT run_id FROM health_checks WHERE bookmark_id = ?1 ORDER BY finished_at DESC LIMIT 1",
                    params![bm_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "unknown".to_string());

            tx.execute(
                "INSERT INTO health_summaries (
                    summary_id, bookmark_id, url, last_run_id, last_checked_at, current_verdict,
                    total_checks, consecutive_failures, first_failed_at, last_failed_at, last_success_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    summary_id,
                    bm_id,
                    data.url,
                    last_run_id,
                    last_checked_at,
                    current_verdict,
                    total_checks as i64,
                    consecutive_failures as i64,
                    first_failed_at,
                    last_failed_at,
                    last_success_at,
                ],
            )?;
            written += 1;
        }

        tx.execute(
            "INSERT OR REPLACE INTO health_schema_version (version, applied_at) VALUES (1, ?1)",
            params![now_str],
        )
        .ok(); // best-effort — schema version row may already exist

        tx.commit()?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::models::BackendObservation;
    use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;

    #[test]
    fn test_db_migration_and_recording() {
        let db = HealthDatabase::in_memory().unwrap();
        let ctx = ProbeContext {
            run_id: "run-1".to_string(),
            check_id: "check-1".to_string(),
            proxy_url: None,
            timeout_secs: 10,
            slow_mode: false,
            allow_private_ips: false,
            history_db_path: None,
            secondary_backends: vec![],
            artifact_dir: None,
        };

        db.record_run_start(&ctx).unwrap();
        let obs = BackendObservation::default();
        db.record_check_result(
            "run-1",
            "check-1",
            "bm-1",
            "https://example.com",
            HealthVerdict::Live,
            &[obs],
        )
        .unwrap();
        db.record_run_finish("run-1").unwrap();

        let history = db.get_url_history("https://example.com").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].2, "live");
    }

    #[test]
    fn content_aware_success_excludes_conflicting_failure_run_from_evidence() {
        let db = HealthDatabase::in_memory().unwrap();
        let ctx = ProbeContext {
            run_id: "run-conflict".to_string(),
            check_id: "check-conflict".to_string(),
            proxy_url: None,
            timeout_secs: 10,
            slow_mode: false,
            allow_private_ips: false,
            history_db_path: None,
            secondary_backends: vec!["curl".to_string()],
            artifact_dir: None,
        };
        db.record_run_start(&ctx).unwrap();
        let missing = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(200),
            error_kind: Some("ContentMissing".to_string()),
            body_hash: Some("removed-body".to_string()),
            ..Default::default()
        };
        let success = BackendObservation {
            backend: "curl".to_string(),
            status_code: Some(200),
            error_kind: None,
            body_hash: Some("real-body".to_string()),
            content_length: Some(9),
            ..Default::default()
        };
        db.record_check_result(
            &ctx.run_id,
            &ctx.check_id,
            "bm-conflict",
            "https://example.com/post/1",
            HealthVerdict::Recovered,
            &[missing, success],
        )
        .unwrap();
        db.record_run_finish(&ctx.run_id).unwrap();

        let evidence = db
            .get_qualifying_run_evidences("bm-conflict", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap();
        assert!(evidence.is_empty());
    }

    #[test]
    fn quarantine_expires_after_one_day() {
        let db = HealthDatabase::in_memory().unwrap();
        for (id, checked_at) in [
            ("recent", Utc::now() - chrono::Duration::minutes(5)),
            ("stale", Utc::now() - chrono::Duration::hours(25)),
        ] {
            db.conn
                .execute(
                    "INSERT INTO health_summaries (
                        summary_id, bookmark_id, url, last_run_id, last_checked_at,
                        current_verdict, total_checks, consecutive_failures
                     ) VALUES (?1, ?1, ?2, ?1, ?3, 'confirmed_dead', 3, 3)",
                    params![id, format!("https://{id}.example"), checked_at.to_rfc3339()],
                )
                .unwrap();
        }

        let candidates = db.get_quarantine_candidates().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "https://recent.example");
    }
}
