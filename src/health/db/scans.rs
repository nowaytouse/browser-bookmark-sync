//! Invocation membership is inspection data, never an independent health check.

use super::{sql_count, HealthDatabase};
use crate::health::{
    models::{BackendObservation, ConfigHashInput, HealthVerdict, ProbeContext, ProbeTarget},
    reporting::target_evidence,
    state_machine::CURRENT_HEALTH_RULE_VERSION,
    targets::ProbeScope,
};
use chrono::Utc;
use rusqlite::{params, types::Type, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HistoricalScanTarget {
    pub scan_id: String,
    pub scope: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub partial: bool,
    pub planned_targets: usize,
    pub config_hash: String,
    pub rule_version: String,
    pub completed: bool,
    pub bookmark_id: String,
    pub url: String,
    /// Exact final-report evidence projection; None means no final result was recorded.
    pub result: Option<serde_json::Value>,
}

impl HealthDatabase {
    /// Atomically register a scope invocation and its eligible targets before any probes.
    pub fn record_scan_start(
        &self,
        scan_id: &str,
        context: &ProbeContext,
        targets: &[ProbeTarget],
        partial: bool,
    ) -> Result<()> {
        let config_hash = ConfigHashInput::from_probe_context(context)
            .stable_hash()
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO health_scans
             (scan_id, started_at, scope, partial, planned_targets, config_hash, rule_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                scan_id,
                Utc::now().to_rfc3339(),
                match context.probe_scope {
                    ProbeScope::Site => "site",
                    ProbeScope::Content => "content",
                },
                partial,
                sql_count(targets.len())?,
                config_hash,
                CURRENT_HEALTH_RULE_VERSION
            ],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO health_scan_targets (scan_id, bookmark_id, original_url)
                 VALUES (?1, ?2, ?3)",
            )?;
            for target in targets {
                insert.execute(params![scan_id, target.bookmark_id, target.original_url])?;
            }
        }
        tx.commit()
    }

    /// Save the final result once, including reuse, without creating another evidence run.
    pub fn record_scan_target(
        &self,
        scan_id: &str,
        bookmark_id: &str,
        url: &str,
        verdict: HealthVerdict,
        observations: &[BackendObservation],
    ) -> Result<()> {
        let json = serde_json::to_string(&target_evidence(bookmark_id, url, verdict, observations))
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let changed = self.conn.execute(
            "UPDATE health_scan_targets SET result_json = ?1
             WHERE scan_id = ?2 AND bookmark_id = ?3 AND original_url = ?4 AND result_json IS NULL
             AND EXISTS (SELECT 1 FROM health_scans WHERE scan_id = ?2 AND completed = 0)",
            params![json, scan_id, bookmark_id, url],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "scan target identity mismatch, already recorded or scan completed".into(),
            ));
        }
        Ok(())
    }

    /// Mark success only after reports were exported and every planned target was recorded.
    pub fn record_scan_finish(&self, scan_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE health_scans SET completed = 1, finished_at = ?1
             WHERE scan_id = ?2 AND completed = 0
             AND planned_targets = (SELECT COUNT(*) FROM health_scan_targets WHERE scan_id = ?2)
             AND NOT EXISTS (SELECT 1 FROM health_scan_targets WHERE scan_id = ?2 AND result_json IS NULL)",
            params![Utc::now().to_rfc3339(), scan_id],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "scan missing, incomplete or already completed".into(),
            ));
        }
        Ok(())
    }

    /// Read exact-URL invocation membership, newest stored first. None means legacy unrecorded.
    pub fn get_url_scan_history(
        &self,
        url: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Option<Vec<HistoricalScanTarget>>> {
        if !(1..=500).contains(&limit) {
            return Err(rusqlite::Error::InvalidParameterName(
                "history limit must be 1..=500".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let tables: i64 = tx.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table'
             AND name IN ('health_scans', 'health_scan_targets')",
            [],
            |row| row.get(0),
        )?;
        if tables == 0 {
            let version = tx.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM health_schema_version",
                [],
                |row| super::read_count(row, 0),
            )?;
            if version >= 5 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "scan history tables are missing from a schema that requires them".into(),
                ));
            }
            tx.commit()?;
            return Ok(None);
        }
        let history = {
            let mut statement = tx.prepare(
                "WITH selected AS (
                    SELECT s.rowid AS record_order, s.* FROM health_scans s
                    WHERE s.scan_id IN (SELECT scan_id FROM health_scan_targets WHERE original_url = ?1)
                    ORDER BY s.rowid DESC LIMIT ?2 OFFSET ?3
                 )
                 SELECT s.scan_id, s.scope, s.started_at, s.finished_at, s.partial, s.planned_targets,
                        s.config_hash, s.rule_version, s.completed, t.bookmark_id, t.original_url, t.result_json
                 FROM selected s JOIN health_scan_targets t ON t.scan_id = s.scan_id
                 WHERE t.original_url = ?1 ORDER BY s.record_order DESC, t.bookmark_id",
            )?;
            let rows = statement.query_map(
                params![url, sql_count(limit)?, sql_count(offset)?],
                |row| {
                    let json: Option<String> = row.get(11)?;
                    let result: Option<serde_json::Value> = json
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                11,
                                Type::Text,
                                Box::new(error),
                            )
                        })?;
                    let target = HistoricalScanTarget {
                        scan_id: row.get(0)?,
                        scope: row.get(1)?,
                        started_at: row.get(2)?,
                        finished_at: row.get(3)?,
                        partial: row.get(4)?,
                        planned_targets: super::read_count(row, 5)?,
                        config_hash: row.get(6)?,
                        rule_version: row.get(7)?,
                        completed: row.get(8)?,
                        bookmark_id: row.get(9)?,
                        url: row.get(10)?,
                        result,
                    };
                    if target.result.as_ref().is_some_and(|result| {
                        result["bookmark_id"].as_str() != Some(target.bookmark_id.as_str())
                            || result["url"].as_str() != Some(target.url.as_str())
                            || result["verdict"].as_str().is_none()
                            || !result["observations"].is_array()
                    }) {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            11,
                            Type::Text,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "stored scan result identity or structure mismatch",
                            )),
                        ));
                    }
                    Ok(target)
                },
            )?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(Some(history))
    }
}
