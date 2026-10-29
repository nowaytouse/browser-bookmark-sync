use crate::health::db::{HealthDatabase, HistoricalCheck, HistoricalScanTarget};
use anyhow::Context;
use serde_json::Value;
use std::path::Path;

pub const PAGE_SIZE: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activity {
    Fresh,
    Reused,
    Pending,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct HistoryRecord {
    pub id: String,
    pub recorded_at: Option<String>,
    pub completed: Option<bool>,
    pub verdict: Option<String>,
    pub rule_version: Option<String>,
    pub partial: bool,
    pub scope: Option<String>,
    pub activity: Activity,
    pub observations: Vec<Value>,
    pub raw: Vec<Value>,
    pub conflicting: bool,
}

#[derive(Debug)]
pub struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub legacy: bool,
    pub has_more: bool,
}

/// Load stored scan invocations; older check-only history is explicitly marked legacy.
pub fn load_page(path: &Path, url: &str, offset: usize) -> Result<HistoryPage, String> {
    let db = HealthDatabase::open_read_only(path)
        .with_context(|| format!("open history database {}", path.display()))
        .map_err(|error| format!("{error:#}"))?;

    // Decide the source from page one even for later offsets, so an empty scan page
    // never silently switches to legacy checks.
    let first_scan_page = db
        .get_url_scan_history(url, 1, 0)
        .context("read scan-history mode")
        .map_err(|error| format!("{error:#}"))?;
    if first_scan_page
        .as_ref()
        .is_some_and(|rows| !rows.is_empty())
    {
        let rows = db
            .get_url_scan_history(url, PAGE_SIZE + 1, offset)
            .context("read scan history")
            .map_err(|error| format!("{error:#}"))?
            .ok_or_else(|| "scan history became unavailable while reading".to_owned())?;
        let mut records = group_scans(rows)?;
        let has_more = records.len() > PAGE_SIZE;
        records.truncate(PAGE_SIZE);
        return Ok(HistoryPage {
            records,
            legacy: false,
            has_more,
        });
    }

    let checks = db
        .get_url_history_details(url, PAGE_SIZE + 1, offset)
        .context("read legacy check history")
        .map_err(|error| format!("{error:#}"))?;
    let has_more = checks.len() > PAGE_SIZE;
    let records = checks
        .into_iter()
        .take(PAGE_SIZE)
        .map(legacy_check)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HistoryPage {
        records,
        legacy: true,
        has_more,
    })
}

fn group_scans(rows: Vec<HistoricalScanTarget>) -> Result<Vec<HistoryRecord>, String> {
    let mut grouped: Vec<(String, Vec<HistoricalScanTarget>)> = Vec::new();
    for row in rows {
        if grouped.last().is_none_or(|(id, _)| id != &row.scan_id) {
            grouped.push((row.scan_id.clone(), Vec::new()));
        }
        if let Some((_, aliases)) = grouped.last_mut() {
            aliases.push(row);
        } else {
            return Err("could not create scan history group".into());
        }
    }
    grouped
        .into_iter()
        .map(|(id, aliases)| scan_record(id, aliases))
        .collect()
}

fn scan_record(id: String, aliases: Vec<HistoricalScanTarget>) -> Result<HistoryRecord, String> {
    let first = aliases
        .first()
        .ok_or_else(|| "empty scan group returned by history database".to_owned())?;
    let mut raw = Vec::with_capacity(aliases.len());
    let mut observations = Vec::new();
    let mut verdicts = Vec::with_capacity(aliases.len());
    let mut rule_versions = Vec::with_capacity(aliases.len());
    let mut activity = Activity::Unknown;
    let all_pending = aliases.iter().all(|alias| alias.result.is_none());

    for alias in &aliases {
        raw.push(serde_json::to_value(alias).map_err(|error| error.to_string())?);
        rule_versions.push(alias.rule_version.clone());
        let verdict = alias
            .result
            .as_ref()
            .and_then(|result| result.get("verdict"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        verdicts.push(verdict);
        if let Some(items) = alias
            .result
            .as_ref()
            .and_then(|result| result.get("observations"))
            .and_then(Value::as_array)
        {
            for item in items {
                let kind = observation_activity(item);
                if kind == Activity::Fresh {
                    activity = Activity::Fresh;
                } else if kind == Activity::Reused && activity != Activity::Fresh {
                    activity = Activity::Reused;
                }
                let mut observation = item.clone();
                if let Some(fields) = observation.as_object_mut() {
                    fields.insert(
                        "bookmark_id".into(),
                        Value::String(alias.bookmark_id.clone()),
                    );
                    fields.insert("url".into(), Value::String(alias.url.clone()));
                }
                observations.push(observation);
            }
        }
    }

    let verdict_values: std::collections::BTreeSet<_> =
        verdicts.iter().filter_map(Option::as_ref).collect();
    let rule_values: std::collections::BTreeSet<_> = rule_versions.iter().collect();
    let conflicting = verdict_values.len() > 1 || rule_values.len() > 1;
    let verdict = if !conflicting && verdicts.iter().all(Option::is_some) {
        verdict_values.first().map(|value| (**value).clone())
    } else {
        None
    };
    let rule_version = if rule_values.len() == 1 && !conflicting {
        rule_values.first().map(|value| (*value).clone())
    } else {
        None
    };
    if all_pending {
        activity = Activity::Pending;
    }

    Ok(HistoryRecord {
        id,
        recorded_at: Some(first.started_at.clone()),
        completed: Some(aliases.iter().all(|alias| alias.completed)),
        verdict,
        rule_version,
        partial: aliases.iter().any(|alias| alias.partial),
        scope: aliases
            .iter()
            .all(|alias| alias.scope == first.scope)
            .then(|| first.scope.clone()),
        activity,
        observations,
        raw,
        conflicting,
    })
}

fn observation_activity(observation: &Value) -> Activity {
    let reused = observation["cache_hit"].as_bool() == Some(true)
        || observation["request_method"].as_str() == Some("REUSE")
        || observation["backend"].as_str() == Some("same_day_resume");
    if reused {
        Activity::Reused
    } else {
        match observation["actual_request"].as_str() {
            Some("yes") => Activity::Fresh,
            Some("no") | Some("unknown") | None => Activity::Unknown,
            Some(_) => Activity::Unknown,
        }
    }
}

fn legacy_check(check: HistoricalCheck) -> Result<HistoryRecord, String> {
    let observations: Vec<Value> = check
        .observations
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    let activity = observations.iter().map(observation_activity).fold(
        Activity::Unknown,
        |current, kind| match (current, kind) {
            (Activity::Fresh, _) | (_, Activity::Fresh) => Activity::Fresh,
            (Activity::Reused, _) | (_, Activity::Reused) => Activity::Reused,
            _ => Activity::Unknown,
        },
    );
    let raw = vec![serde_json::to_value(&check).map_err(|error| error.to_string())?];
    Ok(HistoryRecord {
        id: check.check_id,
        recorded_at: check.run_started_at,
        completed: check.completed,
        verdict: Some(check.verdict),
        rule_version: check.rule_version,
        partial: false,
        scope: None,
        activity,
        observations,
        raw,
        conflicting: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::{
        models::{BackendObservation, HealthVerdict, ProbeContext, ProbeTarget},
        targets::ProbeScope,
    };
    use chrono::Utc;

    const URL: &str = "https://history.example.test/";

    fn probe_context(id: &str) -> ProbeContext {
        ProbeContext {
            run_id: id.into(),
            check_id: format!("{id}-check"),
            proxy_url: None,
            timeout_secs: 5,
            slow_mode: false,
            probe_scope: ProbeScope::Content,
            dns_policy_hash: "dns-policy".into(),
            allow_private_ips: false,
            history_db_path: None,
            secondary_backends: Vec::new(),
            artifact_dir: None,
        }
    }

    fn target(bookmark_id: &str) -> ProbeTarget {
        ProbeTarget {
            bookmark_id: bookmark_id.into(),
            original_url: URL.into(),
            scheme: "https".into(),
            host: "history.example.test".into(),
            port: None,
            registrable_domain: "example.test".into(),
            original_title: None,
        }
    }

    fn observation(
        actual_request: &str,
        method: &str,
        cache_hit: bool,
        backend: &str,
    ) -> BackendObservation {
        BackendObservation {
            backend: backend.into(),
            backend_version: "test".into(),
            run_id: "run".into(),
            check_id: "check".into(),
            source: None,
            route: "direct".into(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            actual_request: actual_request.into(),
            cache_hit,
            request_method: method.into(),
            status_code: Some(200),
            error_kind: None,
            initial_url: URL.into(),
            final_url: None,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            response_time_ms: 1,
            content_type: None,
            content_length: None,
            title: None,
            canonical_url: None,
            body_preview: None,
            body_hash: None,
            header_hash: None,
            ip_addresses: Vec::new(),
            dns_rcode: None,
            resolver_id: None,
            tls_summary: None,
            screenshot_path: None,
            raw_artifact_path: None,
            confidence: 1.0,
        }
    }

    #[test]
    fn scan_history_groups_aliases_and_keeps_activity_distinct_from_checks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.sqlite");
        let db = HealthDatabase::open(&path).unwrap();
        let context = probe_context("scan-fresh");
        db.record_scan_start(
            "scan-fresh",
            &context,
            &[target("bookmark-a"), target("bookmark-b")],
            false,
        )
        .unwrap();
        db.record_scan_target(
            "scan-fresh",
            "bookmark-a",
            URL,
            HealthVerdict::Live,
            &[observation("yes", "GET", false, "native_reqwest")],
        )
        .unwrap();
        db.record_scan_target(
            "scan-fresh",
            "bookmark-b",
            URL,
            HealthVerdict::Live,
            &[observation("no", "REUSE", true, "same_day_resume")],
        )
        .unwrap();
        db.record_scan_finish("scan-fresh").unwrap();

        let context = probe_context("scan-reused");
        db.record_scan_start("scan-reused", &context, &[target("bookmark-c")], false)
            .unwrap();
        db.record_scan_target(
            "scan-reused",
            "bookmark-c",
            URL,
            HealthVerdict::Live,
            &[observation("no", "REUSE", true, "same_day_resume")],
        )
        .unwrap();
        db.record_scan_finish("scan-reused").unwrap();

        let context = probe_context("scan-pending");
        db.record_scan_start("scan-pending", &context, &[target("bookmark-d")], true)
            .unwrap();
        drop(db);

        let page = load_page(&path, URL, 0).unwrap();
        assert!(!page.legacy);
        assert_eq!(page.records.len(), 3);
        assert_eq!(page.records[0].id, "scan-pending");
        assert_eq!(page.records[0].activity, Activity::Pending);
        assert_eq!(page.records[0].completed, Some(false));
        assert_eq!(page.records[0].verdict, None);
        assert_eq!(page.records[1].activity, Activity::Reused);
        assert_eq!(page.records[2].activity, Activity::Fresh);
        assert_eq!(page.records[2].raw.len(), 2);
        assert_eq!(page.records[2].observations.len(), 2);
        assert_eq!(page.records[2].observations[0]["bookmark_id"], "bookmark-a");
        assert_eq!(page.records[2].observations[1]["bookmark_id"], "bookmark-b");
        assert!(!page.records[2].conflicting);
        assert!(!page.has_more);
    }

    #[test]
    fn check_only_history_is_returned_as_legacy_not_scan_membership() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy-check.sqlite");
        let db = HealthDatabase::open(&path).unwrap();
        let context = probe_context("legacy-run");
        db.record_run_start(&context).unwrap();
        db.record_check_result(
            &context.run_id,
            &context.check_id,
            "bookmark-legacy",
            URL,
            HealthVerdict::NeedsReview,
            &[observation("yes", "GET", false, "native_reqwest")],
        )
        .unwrap();
        db.record_run_finish(&context.run_id).unwrap();
        drop(db);

        let page = load_page(&path, URL, 0).unwrap();
        assert!(page.legacy);
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].id, context.check_id);
        assert_eq!(page.records[0].activity, Activity::Fresh);
        assert_eq!(page.records[0].scope, None);
        assert_eq!(page.records[0].verdict.as_deref(), Some("needs_review"));
        assert_eq!(page.records[0].observations.len(), 1);
    }

    #[test]
    fn scan_pagination_counts_groups_and_keeps_scan_mode_on_empty_last_page() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pages.sqlite");
        let db = HealthDatabase::open(&path).unwrap();
        for index in 0..14 {
            let scan_id = format!("scan-{index:02}");
            let context = probe_context(&scan_id);
            let bookmark_id = format!("bookmark-{index:02}");
            db.record_scan_start(&scan_id, &context, &[target(&bookmark_id)], false)
                .unwrap();
            db.record_scan_target(
                &scan_id,
                &bookmark_id,
                URL,
                HealthVerdict::Live,
                &[observation("yes", "GET", false, "native_reqwest")],
            )
            .unwrap();
            db.record_scan_finish(&scan_id).unwrap();
        }
        drop(db);

        let first = load_page(&path, URL, 0).unwrap();
        assert_eq!(first.records.len(), PAGE_SIZE);
        assert!(first.has_more);
        assert!(!first.legacy);

        let last = load_page(&path, URL, PAGE_SIZE).unwrap();
        assert_eq!(last.records.len(), 2);
        assert!(!last.has_more);
        assert!(!last.legacy);

        let empty = load_page(&path, URL, 14).unwrap();
        assert!(empty.records.is_empty());
        assert!(!empty.legacy);
        assert!(!empty.has_more);
    }

    #[test]
    fn corrupt_scan_history_is_an_error_not_legacy_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("corrupt-scan.sqlite");
        let db = HealthDatabase::open(&path).unwrap();

        let legacy = probe_context("legacy-run");
        db.record_run_start(&legacy).unwrap();
        db.record_check_result(
            &legacy.run_id,
            &legacy.check_id,
            "bookmark-legacy",
            URL,
            HealthVerdict::NeedsReview,
            &[observation("yes", "GET", false, "native_reqwest")],
        )
        .unwrap();
        db.record_run_finish(&legacy.run_id).unwrap();

        let scan = probe_context("scan-corrupt");
        db.record_scan_start("scan-corrupt", &scan, &[target("bookmark-scan")], false)
            .unwrap();
        db.record_scan_target(
            "scan-corrupt",
            "bookmark-scan",
            URL,
            HealthVerdict::Live,
            &[observation("yes", "GET", false, "native_reqwest")],
        )
        .unwrap();
        db.record_scan_finish("scan-corrupt").unwrap();
        db.raw_conn_for_test()
            .execute(
                "UPDATE health_scan_targets SET result_json = ?1 WHERE scan_id = ?2",
                rusqlite::params!["not-json", "scan-corrupt"],
            )
            .unwrap();
        drop(db);

        assert!(load_page(&path, URL, 0).is_err());
    }
}
