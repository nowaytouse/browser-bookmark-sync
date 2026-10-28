//! Read-only inspection of stored checks, not recomputation of deletion eligibility.

use super::{sql_count, HealthDatabase};
use crate::health::models::ObservationSource;
use rusqlite::{params, types::Type, Connection, OpenFlags, Result};
use serde::Serialize;
use std::{collections::HashSet, path::Path};

#[derive(Debug, Serialize)]
pub struct HistoricalCheck {
    pub run_id: String,
    pub check_id: String,
    pub bookmark_id: String,
    pub run_started_at: Option<String>,
    pub run_finished_at: Option<String>,
    pub completed: Option<bool>,
    pub config_hash: Option<String>,
    pub rule_version: Option<String>,
    pub verdict: String,
    pub observations: Vec<HistoricalObservation>,
}

#[derive(Debug, Serialize)]
pub struct HistoricalObservation {
    pub attempt_id: String,
    pub backend: String,
    pub backend_version: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub actual_request: String,
    pub cache_hit: bool,
    pub request_method: String,
    pub status_code: Option<u16>,
    pub error_kind: Option<String>,
    pub dns_rcode: Option<String>,
    pub resolver_id: Option<String>,
    pub source: Option<ObservationSource>,
}

impl HealthDatabase {
    /// Open an existing history database without creating, migrating or updating it.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.pragma_update(None, "query_only", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    /// Return a page of exact-URL checks, newest stored check first, with all their attempts.
    /// Raw times and old-rule/incomplete records are displayed, not certified as usable evidence.
    pub fn get_url_history_details(
        &self,
        url: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<HistoricalCheck>> {
        if !(1..=500).contains(&limit) {
            return Err(rusqlite::Error::InvalidParameterName(
                "history limit must be 1..=500".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let columns: HashSet<String> = {
            let mut statement = tx.prepare("PRAGMA table_info(health_attempts)")?;
            let columns = statement.query_map([], |row| row.get(1))?;
            columns.collect::<Result<_>>()?
        };
        // Only these fixed schema identifiers can enter the SQL; values stay parameterized.
        let column = |name: &str| {
            if columns.contains(name) {
                format!("a.{name}")
            } else {
                "NULL".into()
            }
        };
        let sql = format!(
            "WITH selected AS (
                SELECT c.rowid AS record_order, c.run_id, c.check_id, c.bookmark_id, c.run_verdict
                FROM health_checks c WHERE c.original_url = ?1
                ORDER BY c.rowid DESC LIMIT ?2 OFFSET ?3
             )
             SELECT s.run_id, s.check_id, s.bookmark_id, r.started_at, r.finished_at,
                    r.completed, r.config_hash, r.rule_version, s.run_verdict,
                    a.attempt_id, a.backend, a.backend_version, {}, {}, a.actual_request,
                    a.cache_hit, a.method, a.status_code, a.error_kind, {}, {}, {}
             FROM selected s LEFT JOIN health_runs r ON r.run_id = s.run_id
             LEFT JOIN health_attempts a ON a.check_id = s.check_id
             ORDER BY s.record_order DESC, a.attempt_index, a.attempt_id",
            column("started_at"),
            column("finished_at"),
            column("dns_rcode"),
            column("resolver_id"),
            column("source_json")
        );
        let mut statement = tx.prepare(&sql)?;
        let mut rows = statement.query(params![url, sql_count(limit)?, sql_count(offset)?])?;
        let mut checks: Vec<HistoricalCheck> = Vec::new();
        while let Some(row) = rows.next()? {
            let check_id: String = row.get(1)?;
            if checks.last().is_none_or(|check| check.check_id != check_id) {
                checks.push(HistoricalCheck {
                    run_id: row.get(0)?,
                    check_id,
                    bookmark_id: row.get(2)?,
                    run_started_at: row.get(3)?,
                    run_finished_at: row.get(4)?,
                    completed: row.get(5)?,
                    config_hash: row.get(6)?,
                    rule_version: row.get(7)?,
                    verdict: row.get(8)?,
                    observations: Vec::new(),
                });
            }
            if let Some(attempt_id) = row.get::<_, Option<String>>(9)? {
                let source_json: Option<String> = row.get(21)?;
                let source = source_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(21, Type::Text, Box::new(error))
                    })?;
                let observation = HistoricalObservation {
                    attempt_id,
                    backend: row.get(10)?,
                    backend_version: row.get(11)?,
                    started_at: row.get(12)?,
                    finished_at: row.get(13)?,
                    actual_request: row.get(14)?,
                    cache_hit: row.get(15)?,
                    request_method: row.get(16)?,
                    status_code: row.get(17)?,
                    error_kind: row.get(18)?,
                    dns_rcode: row.get(19)?,
                    resolver_id: row.get(20)?,
                    source,
                };
                if let Some(check) = checks.last_mut() {
                    check.observations.push(observation);
                }
            }
        }
        drop(rows);
        drop(statement);
        tx.commit()?;
        Ok(checks)
    }
}
