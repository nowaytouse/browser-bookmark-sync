//! SQLite History Persistence for Health Probe System

use crate::health::models::{BackendObservation, HealthVerdict, ProbeContext};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use std::path::Path;
use tracing::info;

pub struct HealthDatabase {
    pub(crate) conn: Connection,
}

fn read_count(row: &rusqlite::Row<'_>, column: usize) -> SqlResult<usize> {
    let value: i64 = row.get(column)?;
    usize::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn sql_count(value: usize) -> SqlResult<i64> {
    i64::try_from(value).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn current_evidence_timestamp(at: Option<&str>, now: chrono::DateTime<Utc>) -> bool {
    at.and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| {
            crate::health::state_machine::StateMachine::is_current_evidence(
                at.with_timezone(&Utc),
                now,
            )
        })
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

        let current_version = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM health_schema_version",
            [],
            |row| read_count(row, 0),
        )?;

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
                        read_count(row, 2)?,
                        read_count(row, 3)?,
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
            let counter_overflow = || {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
                    "Health summary counter overflow",
                )))
            };
            let new_total = if is_same_run {
                total
            } else {
                total.checked_add(1).ok_or_else(counter_overflow)?
            };
            let (new_cons_fail, new_first_fail, new_last_fail, new_last_succ) = if is_failure {
                (
                    if is_same_run {
                        cons_fail
                    } else {
                        cons_fail.checked_add(1).ok_or_else(counter_overflow)?
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
                    sql_count(new_total)?,
                    sql_count(new_cons_fail)?,
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
                |row| {
                    Ok((
                        read_count(row, 0)?,
                        read_count(row, 1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
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
        Self::get_qualifying_run_evidences_from(
            &self.conn,
            bookmark_id,
            current_config_hash,
            current_rule_version,
        )
    }

    fn get_qualifying_run_evidences_from(
        conn: &Connection,
        bookmark_id: &str,
        current_config_hash: Option<&str>,
        current_rule_version: &str,
    ) -> SqlResult<Vec<crate::health::models::QualifyingRunEvidence>> {
        use crate::health::models::{QualifyingRunEvidence, StrongFailureKind};
        use chrono::DateTime;
        use std::collections::{BTreeMap, BTreeSet};
        let site_scope = bookmark_id.starts_with("site:");

        // 1. Find the last real success timestamp for this bookmark.
        //    Pre-v3 rows have no attempt timestamp and cannot be promoted into evidence.
        let last_real_success: Option<String> = conn
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
               AND (
                   (?4 = 1 AND a.status_code >= 100 AND a.status_code < 600)
                   OR (
                       ?4 = 0
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
                       )
                   )
               )",
                params![
                    bookmark_id,
                    current_config_hash,
                    current_rule_version,
                    site_scope
                ],
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

        let mut stmt = conn.prepare(&sql)?;

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

        fn strong_failure_kind(a: &AttemptRow, site_scope: bool) -> Option<StrongFailureKind> {
            if a.actual_request != "yes" || a.cache_hit != 0 {
                return None;
            }
            if !site_scope {
                if let Some(code) = a.status_code {
                    match code {
                        404 => return Some(StrongFailureKind::Http404),
                        410 => return Some(StrongFailureKind::Http410),
                        _ => {}
                    }
                }
            }
            if let Some(ref err) = a.error_kind {
                if !site_scope && err == "ContentMissing" {
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
            // Structured ambiguity vetoes deletion evidence from another backend.
            if attempts.iter().any(|attempt| {
                attempt.actual_request != "yes"
                    || attempt.cache_hit != 0
                    || (!site_scope
                        && attempt.error_kind.as_deref() == Some("StructuredContentUncertain"))
            }) {
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
            let site_response = site_scope
                && attempts.iter().any(|attempt| {
                    attempt.actual_request == "yes"
                        && attempt.cache_hit == 0
                        && attempt
                            .status_code
                            .is_some_and(|status| (100..600).contains(&status))
                });
            if site_response
                || (!site_scope
                    && (attempts.iter().any(is_content_aware_success)
                        || (!has_content_signal && attempts.iter().any(is_real_success))))
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
                match strong_failure_kind(a, site_scope) {
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

    /// Return current-day quarantine candidates whose latest verdict is `confirmed_dead`.
    /// Probable or expired verdicts always remain review-only.
    /// Returns (url, `current_verdict`, `last_checked_at`) tuples ordered by check time.
    pub fn get_quarantine_candidates(&self) -> SqlResult<Vec<(String, String, String)>> {
        self.get_quarantine_candidates_detailed().map(|rows| {
            rows.into_iter()
                .map(|(_, url, _, verdict, checked_at)| (url, verdict, checked_at))
                .collect()
        })
    }

    /// Return the identity fields needed to build an auditable quarantine selection.
    pub fn get_quarantine_candidates_detailed(
        &self,
    ) -> SqlResult<Vec<(String, String, String, String, String)>> {
        let now = Utc::now();
        let cutoff = (now - chrono::Duration::hours(24)).to_rfc3339();
        let mut stmt = self.conn.prepare(
            "SELECT bookmark_id, url, last_run_id, current_verdict, last_checked_at
             FROM health_summaries
             WHERE current_verdict = 'confirmed_dead'
               AND julianday(last_checked_at) >= julianday(?1)
             ORDER BY last_checked_at DESC",
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut results = Vec::new();
        for r in rows {
            let candidate = r?;
            if current_evidence_timestamp(Some(&candidate.4), now) {
                results.push(candidate);
            }
        }
        Ok(results)
    }

    /// Re-check a quarantine selection item against the latest 24-hour database state.
    pub fn verify_quarantine_item(
        &self,
        run_id: &str,
        bookmark_id: &str,
        url: &str,
        now: chrono::DateTime<Utc>,
    ) -> SqlResult<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT s.last_checked_at, c.finished_at, r.finished_at
                 FROM health_summaries s
                 JOIN health_checks c ON c.bookmark_id = s.bookmark_id
                      AND c.run_id = s.last_run_id AND c.original_url = s.url
                 JOIN health_runs r ON r.run_id = c.run_id
                 WHERE s.bookmark_id = ?1
                   AND s.last_run_id = ?2
                   AND s.url = ?3
                   AND s.current_verdict = 'confirmed_dead'
                   AND c.run_verdict = 'confirmed_dead'
                   AND r.completed = 1
                   AND r.rule_version = ?4",
        )?;
        let matches = stmt.query_map(
            params![
                bookmark_id,
                run_id,
                url,
                crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION
            ],
            |row| Ok([row.get::<_, Option<String>>(0)?, row.get(1)?, row.get(2)?]),
        )?;
        for times in matches {
            // Parse exact RFC3339 instants: SQLite julianday rounds sub-millisecond
            // boundaries and a fresh summary cannot make an invalid check/run fresh.
            if times?
                .iter()
                .all(|at| current_evidence_timestamp(at.as_deref(), now))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Re-check a site-scoped selection item using its representative URL.
    pub fn verify_quarantine_site_item(
        &self,
        run_id: &str,
        bookmark_id: &str,
        url: &str,
        now: chrono::DateTime<Utc>,
    ) -> SqlResult<bool> {
        self.verify_quarantine_item(run_id, bookmark_id, url, now)
    }

    /// Return counts of bookmarks grouped by `current_verdict` in `health_summaries`.
    pub fn get_verdict_counts(&self) -> SqlResult<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT current_verdict, COUNT(*)
             FROM health_summaries
             GROUP BY current_verdict
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, read_count(row, 1)?)))?;
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

    /// Rebuild `health_summaries` from current, completed raw probe history.
    ///
    /// Per plan §10: "任何 summary 必须可以通过 `health rebuild-summary` 从 attempts/checks 完整重建。"
    ///
    /// For each bookmark, its latest completed run using the current rule selects the active
    /// config hash. Only real, uncached attempts from completed runs using that rule/config in
    /// the rolling 24-hour window participate. Strong-failure counters and death verdicts are
    /// rebuilt from qualifying evidence rather than trusting cached check verdicts.
    ///
    /// This wipes the summary table and recomputes per-bookmark aggregates atomically.
    /// Returns the number of summary rows written.
    pub fn rebuild_summary_from_checks(&self) -> SqlResult<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let cutoff = (now - chrono::Duration::hours(24)).to_rfc3339();
        let current_rule_version = crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;

        // Group current checks by bookmark. MAX(attempt.started_at) is the real check time;
        // check/run timestamps are not substitutes for evidence timestamps.
        use std::collections::BTreeMap;
        struct CheckRow {
            run_id: String,
            verdict: String,
            checked_at: String,
            config_hash: String,
        }
        struct BmData {
            url: String,
            rows: Vec<CheckRow>,
        }
        let mut by_bm: BTreeMap<String, BmData> = BTreeMap::new();

        {
            let mut stmt = tx.prepare(
                "SELECT
                   c.bookmark_id,
                   c.original_url,
                   c.run_id,
                   c.run_verdict,
                   MAX(a.started_at) AS checked_at,
                   r.config_hash
                 FROM health_checks c
                 JOIN health_runs r ON r.run_id = c.run_id
                 JOIN health_attempts a ON a.check_id = c.check_id
                 WHERE r.completed = 1
                   AND r.finished_at IS NOT NULL
                   AND c.finished_at IS NOT NULL
                   AND r.rule_version = ?1
                   AND r.config_hash IS NOT NULL
                   AND TRIM(r.config_hash) <> ''
                   AND a.actual_request = 'yes'
                   AND a.cache_hit = 0
                   AND a.started_at IS NOT NULL
                   AND julianday(a.started_at) >= julianday(?2)
                   AND julianday(a.started_at) <= julianday(?3)
                 GROUP BY c.check_id, c.bookmark_id, c.original_url, c.run_id,
                          c.run_verdict, r.config_hash
                 ORDER BY c.bookmark_id, julianday(checked_at) ASC, c.run_id ASC",
            )?;

            let rows = stmt.query_map(params![current_rule_version, cutoff, now_str], |row| {
                Ok((
                    row.get::<_, String>(0)?, // bookmark_id
                    row.get::<_, String>(1)?, // original_url
                    CheckRow {
                        run_id: row.get(2)?,
                        verdict: row.get(3)?,
                        checked_at: row.get(4)?,
                        config_hash: row.get(5)?,
                    },
                ))
            })?;

            for r in rows {
                let (bm_id, url, check_row) = r?;
                let entry = by_bm.entry(bm_id).or_insert_with(|| BmData {
                    url: url.clone(),
                    rows: Vec::new(),
                });
                entry.url = url;
                entry.rows.push(check_row);
            }
        }

        // Summaries are a cache. Delete only after all authoritative reads succeed, while the
        // same transaction still protects the snapshot used for this rebuild.
        tx.execute("DELETE FROM health_summaries", [])?;
        let mut written = 0usize;

        for (bm_id, data) in &by_bm {
            let Some(latest) = data.rows.last() else {
                continue;
            };
            let config_hash = latest.config_hash.as_str();
            let current_rows: Vec<_> = data
                .rows
                .iter()
                .filter(|row| row.config_hash == config_hash)
                .collect();
            let total_checks = current_rows.len();
            let mut evidences = Self::get_qualifying_run_evidences_from(
                &tx,
                bm_id,
                Some(config_hash),
                current_rule_version,
            )?;
            evidences.retain(|evidence| {
                let age = now.signed_duration_since(evidence.strong_failure_at);
                age >= chrono::Duration::zero() && age <= chrono::Duration::hours(24)
            });
            evidences.sort_by_key(|evidence| evidence.completed_at);

            let stored_verdict =
                HealthVerdict::from_db_str(&latest.verdict).unwrap_or(HealthVerdict::NeedsReview);
            let latest_is_qualifying = evidences
                .iter()
                .any(|evidence| evidence.run_id == latest.run_id);

            // Death states are rebuilt exclusively from current qualifying evidence. A cached
            // dead verdict without matching evidence is always downgraded to manual review.
            let current_verdict = if latest_is_qualifying {
                let confirmed = if evidences.len() >= 3 {
                    let families: std::collections::BTreeSet<&str> = evidences
                        .iter()
                        .flat_map(|evidence| evidence.verifier_families.iter().map(String::as_str))
                        .collect();
                    let all_nxdomain = evidences.iter().all(|evidence| {
                        evidence
                            .strong_failure_kinds
                            .contains(&crate::health::models::StrongFailureKind::ConfirmedNxDomain)
                    });
                    let last_three_content_missing =
                        evidences.iter().rev().take(3).all(|evidence| {
                            evidence
                                .strong_failure_kinds
                                .contains(&crate::health::models::StrongFailureKind::ContentMissing)
                        });
                    families.len() >= 2 || all_nxdomain || last_three_content_missing
                } else {
                    false
                };

                if confirmed {
                    HealthVerdict::ConfirmedDead
                } else if evidences.len() >= 2 {
                    HealthVerdict::ProbableDead
                } else {
                    HealthVerdict::NeedsReview
                }
            } else if matches!(
                stored_verdict,
                HealthVerdict::ProbableDead | HealthVerdict::ConfirmedDead
            ) {
                HealthVerdict::NeedsReview
            } else {
                stored_verdict
            };

            let first_failed_at = evidences
                .iter()
                .map(|evidence| evidence.strong_failure_at)
                .min()
                .map(|at| at.to_rfc3339());
            let last_failed_at = evidences
                .iter()
                .map(|evidence| evidence.strong_failure_at)
                .max()
                .map(|at| at.to_rfc3339());
            let last_success_at = current_rows
                .iter()
                .rev()
                .find(|row| matches!(row.verdict.as_str(), "live" | "recovered"))
                .map(|row| row.checked_at.clone());

            let summary_id = uuid::Uuid::new_v4().to_string();

            tx.execute(
                "INSERT INTO health_summaries (
                    summary_id, bookmark_id, url, last_run_id, last_checked_at, current_verdict,
                    total_checks, consecutive_failures, first_failed_at, last_failed_at,
                    last_success_at, rule_version, config_hash
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    summary_id,
                    bm_id,
                    data.url,
                    latest.run_id,
                    latest.checked_at,
                    current_verdict.as_str(),
                    sql_count(total_checks)?,
                    sql_count(evidences.len())?,
                    first_failed_at,
                    last_failed_at,
                    last_success_at,
                    current_rule_version,
                    config_hash,
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
    fn rejects_negative_database_counts() {
        let db = HealthDatabase::in_memory().unwrap();
        let result = db.conn.query_row("SELECT -1", [], |row| read_count(row, 0));

        assert!(matches!(
            result,
            Err(rusqlite::Error::IntegralValueOutOfRange(0, -1))
        ));
    }

    #[test]
    fn migration_rejects_negative_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE health_schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             INSERT INTO health_schema_version VALUES (-1, 'invalid');",
        )
        .unwrap();

        assert!(matches!(
            HealthDatabase { conn }.migrate(),
            Err(rusqlite::Error::IntegralValueOutOfRange(0, -1))
        ));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn summary_counter_overflow_rolls_back_check() {
        for column in ["total_checks", "consecutive_failures"] {
            let db = HealthDatabase::in_memory().unwrap();
            db.record_check_result(
                "run-1",
                "check-1",
                "bm-1",
                "https://example.com",
                HealthVerdict::TemporaryFailure,
                &[],
            )
            .unwrap();
            db.conn
                .execute(
                    &format!("UPDATE health_summaries SET {column} = ?1"),
                    params![i64::MAX],
                )
                .unwrap();

            let result = db.record_check_result(
                "run-2",
                "check-2",
                "bm-1",
                "https://example.com",
                HealthVerdict::TemporaryFailure,
                &[],
            );
            assert!(matches!(
                result,
                Err(rusqlite::Error::ToSqlConversionFailure(_))
            ));
            let stored: (i64, i64) = db
                .conn
                .query_row(
                    &format!(
                        "SELECT {column}, (SELECT COUNT(*) FROM health_checks) FROM health_summaries"
                    ),
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(stored, (i64::MAX, 1));
        }
    }

    #[test]
    fn test_db_migration_and_recording() {
        let db = HealthDatabase::in_memory().unwrap();
        let ctx = ProbeContext {
            run_id: "run-1".to_string(),
            check_id: "check-1".to_string(),
            proxy_url: None,
            timeout_secs: 10,
            slow_mode: false,
            probe_scope: crate::health::targets::ProbeScope::Content,
            dns_policy_hash: "test-dns-policy".to_string(),
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
            probe_scope: crate::health::targets::ProbeScope::Content,
            dns_policy_hash: "test-dns-policy".to_string(),
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
    fn structured_uncertainty_excludes_conflicting_failure_run_from_evidence() {
        for status in [200, 404] {
            for uncertain in [false, true] {
                let db = HealthDatabase::in_memory().unwrap();
                let ctx = ProbeContext {
                    run_id: "run-structured-conflict".to_string(),
                    check_id: "check-structured-conflict".to_string(),
                    proxy_url: None,
                    timeout_secs: 10,
                    slow_mode: false,
                    probe_scope: crate::health::targets::ProbeScope::Content,
                    dns_policy_hash: "test-dns-policy".to_string(),
                    allow_private_ips: false,
                    history_db_path: None,
                    secondary_backends: vec!["curl".to_string()],
                    artifact_dir: None,
                };
                db.record_run_start(&ctx).unwrap();
                let primary = BackendObservation {
                    backend: "native_reqwest".to_string(),
                    status_code: Some(200),
                    error_kind: Some(
                        if uncertain {
                            "StructuredContentUncertain"
                        } else {
                            "ContentMissing"
                        }
                        .to_string(),
                    ),
                    body_hash: Some("structured-body".to_string()),
                    content_length: Some(16),
                    ..Default::default()
                };
                let secondary = BackendObservation {
                    backend: "curl".to_string(),
                    status_code: Some(status),
                    error_kind: Some("ContentMissing".to_string()),
                    body_hash: Some("missing-body".to_string()),
                    content_length: Some(12),
                    ..Default::default()
                };
                db.record_check_result(
                    &ctx.run_id,
                    &ctx.check_id,
                    "bm-structured-conflict",
                    "https://www.youtube.com/watch?v=fixture",
                    HealthVerdict::NeedsReview,
                    &[primary, secondary],
                )
                .unwrap();
                db.record_run_finish(&ctx.run_id).unwrap();

                let evidence = db
                    .get_qualifying_run_evidences(
                        "bm-structured-conflict",
                        None,
                        CURRENT_HEALTH_RULE_VERSION,
                    )
                    .unwrap();
                assert_eq!(
                    evidence.len(),
                    usize::from(!uncertain),
                    "secondary HTTP {status}, structured uncertainty={uncertain}"
                );
            }
        }
    }

    #[test]
    fn quarantine_time_window_is_exact_for_summary_check_and_completed_run() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-25T12:00:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);
        let cases = [
            ((now + chrono::Duration::hours(1)).to_rfc3339(), false),
            ((now + chrono::Duration::nanoseconds(1)).to_rfc3339(), false),
            (now.to_rfc3339(), true),
            ((now - chrono::Duration::hours(24)).to_rfc3339(), true),
            (
                (now - chrono::Duration::hours(24) - chrono::Duration::nanoseconds(1)).to_rfc3339(),
                false,
            ),
            ("2026-10-25T20:00:00.123456789+08:00".to_string(), true),
            ("not a timestamp".to_string(), false),
            (String::new(), false),
        ];
        for site in [false, true] {
            for field in ["summary", "check", "run"] {
                for (timestamp, expected) in &cases {
                    let db = HealthDatabase::in_memory().unwrap();
                    let id = if site {
                        "site:fixture.example"
                    } else {
                        "bookmark"
                    };
                    let current = now.to_rfc3339();
                    let started = (now - chrono::Duration::hours(25)).to_rfc3339();
                    let summary_at = if field == "summary" {
                        timestamp
                    } else {
                        &current
                    };
                    let check_at = if field == "check" {
                        timestamp
                    } else {
                        &current
                    };
                    let run_at = if field == "run" { timestamp } else { &current };
                    db.conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,rule_version) VALUES ('run',?1,?2,1,'standard',?3)", params![started, run_at, CURRENT_HEALTH_RULE_VERSION]).unwrap();
                    db.conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES ('check','run',?1,'https://fixture.example/',?2,?3,'confirmed_dead')", params![id, started, check_at]).unwrap();
                    db.conn.execute("INSERT INTO health_summaries (summary_id,bookmark_id,url,last_run_id,last_checked_at,current_verdict,total_checks,consecutive_failures) VALUES ('summary',?1,'https://fixture.example/','run',?2,'confirmed_dead',3,3)", params![id, summary_at]).unwrap();
                    let eligible = if site {
                        db.verify_quarantine_site_item("run", id, "https://fixture.example/", now)
                    } else {
                        db.verify_quarantine_item("run", id, "https://fixture.example/", now)
                    }
                    .unwrap();
                    assert_eq!(eligible, *expected, "site={site}, {field}={timestamp}");
                }
            }
        }
    }

    #[test]
    fn quarantine_expires_after_one_day() {
        let db = HealthDatabase::in_memory().unwrap();
        for (id, checked_at) in [
            ("recent", Utc::now() - chrono::Duration::minutes(5)),
            ("stale", Utc::now() - chrono::Duration::hours(25)),
            ("future", Utc::now() + chrono::Duration::hours(1)),
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

    #[test]
    fn quarantine_excludes_probable_dead() {
        let db = HealthDatabase::in_memory().unwrap();
        let checked_at = Utc::now().to_rfc3339();
        for (id, verdict) in [
            ("confirmed", "confirmed_dead"),
            ("probable", "probable_dead"),
        ] {
            db.conn
                .execute(
                    "INSERT INTO health_summaries (
                        summary_id, bookmark_id, url, last_run_id, last_checked_at,
                        current_verdict, total_checks, consecutive_failures
                     ) VALUES (?1, ?1, ?2, ?1, ?3, ?4, 3, 3)",
                    params![id, format!("https://{id}.example"), checked_at, verdict],
                )
                .unwrap();
        }

        let candidates = db.get_quarantine_candidates().unwrap();
        assert_eq!(
            candidates,
            vec![(
                "https://confirmed.example".to_string(),
                "confirmed_dead".to_string(),
                checked_at,
            )]
        );
    }

    #[test]
    fn rebuild_summary_uses_only_current_completed_qualifying_evidence() {
        let db = HealthDatabase::in_memory().unwrap();
        let now = Utc::now();
        let bookmark_id = "bm-rebuild";
        let url = "https://example.com/video/1";

        let insert_failure = |run_id: &str,
                              hours_ago: i64,
                              completed: bool,
                              rule_version: &str,
                              config_hash: &str,
                              backend: &str| {
            let at = (now - chrono::Duration::hours(hours_ago)).to_rfc3339();
            let check_id = format!("check-{run_id}");
            db.conn
                .execute(
                    "INSERT INTO health_runs (
                        run_id, started_at, finished_at, completed, mode,
                        config_hash, rule_version
                     ) VALUES (?1, ?2, ?2, ?3, 'standard', ?4, ?5)",
                    params![run_id, at, i64::from(completed), config_hash, rule_version],
                )
                .unwrap();
            db.conn
                .execute(
                    "INSERT INTO health_checks (
                        check_id, run_id, bookmark_id, original_url,
                        started_at, finished_at, run_verdict
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'confirmed_dead')",
                    params![check_id, run_id, bookmark_id, url, at],
                )
                .unwrap();
            db.conn
                .execute(
                    "INSERT INTO health_attempts (
                        attempt_id, check_id, backend, backend_version, route,
                        attempt_index, actual_request, cache_hit, method, status_code,
                        initial_url, redirect_count, response_time_ms, confidence,
                        started_at, finished_at
                     ) VALUES (?1, ?2, ?3, 'test', 'direct', 0, 'yes', 0,
                               'GET', 404, ?4, 0, 1, 1.0, ?5, ?5)",
                    params![format!("attempt-{run_id}"), check_id, backend, url, at],
                )
                .unwrap();
        };

        insert_failure(
            "valid-1",
            3,
            true,
            CURRENT_HEALTH_RULE_VERSION,
            "current-config",
            "native_reqwest",
        );
        insert_failure(
            "valid-2",
            2,
            true,
            CURRENT_HEALTH_RULE_VERSION,
            "current-config",
            "curl",
        );
        insert_failure(
            "valid-3",
            1,
            true,
            CURRENT_HEALTH_RULE_VERSION,
            "current-config",
            "native_reqwest",
        );
        insert_failure(
            "old-config",
            4,
            true,
            CURRENT_HEALTH_RULE_VERSION,
            "old-config",
            "curl",
        );
        insert_failure(
            "stale",
            25,
            true,
            CURRENT_HEALTH_RULE_VERSION,
            "current-config",
            "curl",
        );
        insert_failure(
            "old-rule",
            0,
            true,
            "obsolete-rule",
            "current-config",
            "curl",
        );
        insert_failure(
            "incomplete",
            0,
            false,
            CURRENT_HEALTH_RULE_VERSION,
            "current-config",
            "curl",
        );
        db.conn
            .execute(
                "UPDATE health_checks SET run_verdict = 'needs_review'
                 WHERE run_id LIKE 'valid-%'",
                [],
            )
            .unwrap();

        assert_eq!(db.rebuild_summary_from_checks().unwrap(), 1);
        let rebuilt: (String, String, usize, usize, String, String) = db
            .conn
            .query_row(
                "SELECT last_run_id, current_verdict, total_checks,
                        consecutive_failures, rule_version, config_hash
                 FROM health_summaries
                 WHERE bookmark_id = ?1",
                params![bookmark_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        read_count(row, 2)?,
                        read_count(row, 3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            rebuilt,
            (
                "valid-3".to_string(),
                "confirmed_dead".to_string(),
                3,
                3,
                CURRENT_HEALTH_RULE_VERSION.to_string(),
                "current-config".to_string(),
            )
        );
    }
}
