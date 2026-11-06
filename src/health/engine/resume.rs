//! Reuse only attributable, recent positive source evidence.

use super::{
    should_retry, BackendObservation, HealthEngine, HealthVerdict, ObservationSource, ProbeContext,
    ProbeTarget,
};
use chrono::{DateTime, Utc};

impl HealthEngine {
    pub(super) fn load_same_day_positive_results(
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
        let now = Utc::now();
        let cutoff = now - chrono::Duration::hours(24);
        let mut statement = db
            .conn
            .prepare(
                "SELECT c.bookmark_id, c.original_url, c.run_verdict, r.run_id,
                        r.config_hash, r.rule_version, c.check_id,
                        a.started_at, a.finished_at
                 FROM health_checks c
                 JOIN health_runs r ON r.run_id = c.run_id
                 LEFT JOIN health_attempts a ON a.check_id = c.check_id
                     AND a.actual_request = 'yes' AND a.cache_hit = 0
                 WHERE julianday(COALESCE(r.finished_at, r.started_at)) >= julianday(?1)
                 ORDER BY julianday(COALESCE(r.finished_at, r.started_at)) DESC,
                          c.rowid DESC, a.attempt_index, a.attempt_id",
            )
            .map_err(|error| format!("Resume query preparation failed: {error}"))?;
        let rows = statement
            .query_map(rusqlite::params![cutoff.to_rfc3339()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|error| format!("Resume query failed: {error}"))?;

        struct ResumeCandidate {
            verdict: HealthVerdict,
            source: ObservationSource,
            observations: Vec<BackendObservation>,
            valid_times: bool,
        }

        let mut latest_checks = std::collections::HashMap::new();
        let mut candidates = std::collections::HashMap::new();
        for row in rows {
            let (
                bookmark_id,
                url,
                verdict_text,
                run_id,
                row_config_hash,
                rule_version,
                check_id,
                started_at,
                finished_at,
            ) = row.map_err(|error| format!("Resume row decode failed: {error}"))?;
            let key = (bookmark_id.clone(), url.clone());
            if !requested.contains(&key) {
                continue;
            }

            match latest_checks.entry(key.clone()) {
                std::collections::hash_map::Entry::Occupied(latest)
                    if latest.get() != &check_id =>
                {
                    continue;
                }
                std::collections::hash_map::Entry::Vacant(latest) => {
                    // Mark the latest check before validating it. An invalid or
                    // unresolved check must not reveal an older positive result.
                    latest.insert(check_id.clone());
                    let (Some(row_config_hash), Some(rule_version)) =
                        (row_config_hash, rule_version)
                    else {
                        continue;
                    };
                    if row_config_hash != config_hash
                        || rule_version != crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION
                        || run_id.trim().is_empty()
                        || check_id.trim().is_empty()
                    {
                        continue;
                    }
                    let Some(verdict) = HealthVerdict::from_db_str(&verdict_text) else {
                        continue;
                    };
                    if should_retry(verdict) {
                        continue;
                    }
                    candidates.insert(
                        key.clone(),
                        ResumeCandidate {
                            verdict,
                            source: ObservationSource {
                                run_id,
                                check_id,
                                config_hash: row_config_hash,
                                rule_version,
                            },
                            observations: Vec::new(),
                            valid_times: true,
                        },
                    );
                }
                std::collections::hash_map::Entry::Occupied(_) => {}
            }

            // A positive result is safe to reuse even when the enclosing run was interrupted:
            // it can only preserve a bookmark, never promote one into the deletion set.
            let Some(candidate) = candidates.get_mut(&key) else {
                continue;
            };
            let parse_time = |value: Option<&str>| {
                value
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|date| date.with_timezone(&Utc))
            };
            let source_times = parse_time(started_at.as_deref())
                .zip(parse_time(finished_at.as_deref()))
                .filter(|(started, finished)| {
                    cutoff <= *started && *started <= *finished && *finished <= now
                });
            let Some((started_at, finished_at)) = source_times else {
                // Do not discard only a damaged row and reuse its healthy peers.
                candidate.valid_times = false;
                continue;
            };
            candidate.observations.push(BackendObservation {
                backend: "same_day_resume".to_string(),
                backend_version: env!("CARGO_PKG_VERSION").to_string(),
                run_id: candidate.source.run_id.clone(),
                check_id: candidate.source.check_id.clone(),
                source: Some(candidate.source.clone()),
                route: "history".to_string(),
                started_at,
                finished_at,
                actual_request: "no".to_string(),
                cache_hit: true,
                request_method: "REUSE".to_string(),
                initial_url: url.clone(),
                confidence: 1.0,
                ..Default::default()
            });
        }
        let rejected_sources = candidates
            .values()
            .filter(|candidate| !candidate.valid_times || candidate.observations.is_empty())
            .count();
        if rejected_sources > 0 {
            tracing::warn!(
                rejected_sources,
                "Re-probing positive history with missing or invalid real source timestamps"
            );
        }
        Ok(candidates
            .into_iter()
            .filter(|(_, candidate)| candidate.valid_times && !candidate.observations.is_empty())
            .map(|((bookmark_id, url), candidate)| {
                (
                    (bookmark_id.clone(), url.clone()),
                    (bookmark_id, url, candidate.verdict, candidate.observations),
                )
            })
            .collect())
    }
}
