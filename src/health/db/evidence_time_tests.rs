//! Persisted time corruption must not manufacture qualifying death evidence.

use super::*;
use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;
use crate::health::targets::ProbeScope;

pub(super) fn record_missing_run(db: &HealthDatabase, run_id: &str, backends: &[&str]) {
    let context = ProbeContext {
        run_id: run_id.into(),
        check_id: run_id.into(),
        proxy_url: None,
        timeout_secs: 10,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        dns_policy_hash: "synthetic".into(),
        allow_private_ips: false,
        history_db_path: None,
        secondary_backends: vec!["curl".into()],
        artifact_dir: None,
    };
    db.record_run_start(&context).unwrap();
    let missing = |backend: &str| BackendObservation {
        backend: backend.into(),
        status_code: Some(404),
        ..Default::default()
    };
    db.record_check_result(
        run_id,
        run_id,
        "bookmark",
        "https://fixture.example.test/article",
        HealthVerdict::ProbableDead,
        &backends
            .iter()
            .map(|backend| missing(backend))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    db.record_run_finish(run_id).unwrap();
}

#[test]
fn malformed_or_future_run_completion_never_becomes_qualifying_history() {
    let future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    for completion in [
        None,
        Some(""),
        Some("invalid-timestamp"),
        Some(future.as_str()),
    ] {
        let db = HealthDatabase::in_memory().unwrap();
        record_missing_run(&db, "run", &["native-reqwest", "curl"]);
        assert_eq!(
            db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
                .unwrap()
                .len(),
            1,
            "complete real-failure control must remain eligible"
        );
        db.conn
            .execute(
                "UPDATE health_runs SET finished_at = ?1 WHERE run_id = 'run'",
                params![completion],
            )
            .unwrap();
        let evidence = db
            .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap();
        assert!(
            evidence.is_empty(),
            "invalid completion {completion:?} manufactured evidence: {evidence:?}"
        );
    }
}

#[test]
fn invalid_supporting_attempt_cannot_supply_a_second_verifier_family() {
    use crate::health::state_machine::{BookmarkHistoryStats, StateMachine};
    for invalid_time in ["future", "after-completion"] {
        let db = HealthDatabase::in_memory().unwrap();
        for (run, backends) in [
            ("old", vec!["native-reqwest", "curl"]),
            ("middle", vec!["native-reqwest"]),
            ("current", vec!["native-reqwest"]),
        ] {
            record_missing_run(&db, run, &backends);
        }
        let current = BackendObservation {
            status_code: Some(404),
            ..Default::default()
        };
        let resolve = || {
            let evidence = db
                .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
                .unwrap();
            StateMachine::resolve_final_verdict(
                std::slice::from_ref(&current),
                &BookmarkHistoryStats {
                    qualifying_evidences: evidence,
                    ..Default::default()
                },
                Utc::now(),
            )
        };
        assert_eq!(
            resolve(),
            HealthVerdict::ConfirmedDead,
            "three completed runs and two real verifier families remain usable"
        );
        let completed: String = db
            .conn
            .query_row(
                "SELECT finished_at FROM health_runs WHERE run_id = 'old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let invalid = if invalid_time == "future" {
            Utc::now() + chrono::Duration::hours(1)
        } else {
            chrono::DateTime::parse_from_rfc3339(&completed)
                .unwrap()
                .with_timezone(&Utc)
                + chrono::Duration::nanoseconds(1)
        };
        db.conn.execute("UPDATE health_attempts SET started_at = ?1 WHERE check_id = 'old' AND backend = 'curl'", params![invalid.to_rfc3339()]).unwrap();
        assert_eq!(
            resolve(),
            HealthVerdict::ProbableDead,
            "{invalid_time} secondary probe must not help confirm death"
        );
    }
}

#[test]
fn malformed_conflicting_attempt_is_not_silently_dropped_from_a_failure_run() {
    for timestamp in [None, Some("invalid-timestamp")] {
        let db = HealthDatabase::in_memory().unwrap();
        record_missing_run(&db, "run", &["native-reqwest", "curl"]);
        db.conn.execute("UPDATE health_attempts SET started_at = ?1, status_code = 200, body_hash = 'real-content', content_length = 10 WHERE backend = 'curl'", params![timestamp]).unwrap();
        assert!(
            db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
                .unwrap()
                .is_empty(),
            "an undated conflicting success cannot disappear and leave a qualifying failure run"
        );
    }
}

#[test]
fn invalid_second_dns_timestamp_cannot_manufacture_resolver_agreement() {
    use crate::health::models::StrongFailureKind;
    for invalid_time in ["future", "after-completion"] {
        let db = HealthDatabase::in_memory().unwrap();
        record_missing_run(&db, "dns", &["native-reqwest", "curl"]);
        db.conn.execute("UPDATE health_attempts SET status_code = NULL, dns_rcode = 'NXDOMAIN', resolver_id = CASE backend WHEN 'native-reqwest' THEN 'cloudflare' ELSE 'alidns' END", []).unwrap();
        let evidence = db
            .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap();
        assert_eq!(evidence.len(), 1);
        assert!(evidence[0]
            .strong_failure_kinds
            .contains(&StrongFailureKind::ConfirmedNxDomain));
        let completed: String = db
            .conn
            .query_row(
                "SELECT finished_at FROM health_runs WHERE run_id = 'dns'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let invalid = if invalid_time == "future" {
            Utc::now() + chrono::Duration::hours(1)
        } else {
            chrono::DateTime::parse_from_rfc3339(&completed)
                .unwrap()
                .with_timezone(&Utc)
                + chrono::Duration::nanoseconds(1)
        };
        db.conn
            .execute(
                "UPDATE health_attempts SET started_at = ?1 WHERE backend = 'curl'",
                params![invalid.to_rfc3339()],
            )
            .unwrap();
        assert!(
            db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
                .unwrap()
                .is_empty(),
            "{invalid_time} resolver cannot complete agreement"
        );
    }
}

pub(super) fn set_probe_time(
    db: &HealthDatabase,
    run: &str,
    at: chrono::DateTime<Utc>,
    offset: chrono::FixedOffset,
) {
    let started = at.with_timezone(&offset).to_rfc3339();
    let finished = (at + chrono::Duration::seconds(1))
        .with_timezone(&offset)
        .to_rfc3339();
    db.conn
        .execute(
            "UPDATE health_attempts SET started_at = ?1, finished_at = ?2 WHERE check_id = ?3",
            params![started, finished, run],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE health_checks SET started_at = ?1, finished_at = ?2 WHERE check_id = ?3",
            params![started, finished, run],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE health_runs SET started_at = ?1, finished_at = ?2 WHERE run_id = ?3",
            params![started, finished, run],
        )
        .unwrap();
}

#[test]
fn latest_success_is_ordered_by_instant_not_rfc3339_text() {
    let db = HealthDatabase::in_memory().unwrap();
    let now = Utc::now();
    let utc = chrono::FixedOffset::east_opt(0).unwrap();
    for (run, minutes) in [("failure-1", 90), ("failure-2", 60), ("failure-3", 30)] {
        record_missing_run(&db, run, &["native-reqwest", "curl"]);
        set_probe_time(&db, run, now - chrono::Duration::minutes(minutes), utc);
    }
    assert_eq!(
        db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap()
            .len(),
        3
    );
    for (run, at, offset) in [
        (
            "older-success",
            now - chrono::Duration::hours(2),
            chrono::FixedOffset::east_opt(8 * 3600).unwrap(),
        ),
        ("latest-success", now - chrono::Duration::minutes(5), utc),
    ] {
        record_missing_run(&db, run, &["native-reqwest"]);
        set_probe_time(&db, run, at, offset);
        db.conn.execute("UPDATE health_attempts SET status_code = 200, body_hash = 'valid-content', content_length = 100 WHERE check_id = ?1", params![run]).unwrap();
    }
    assert!(db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION).unwrap().is_empty(), "the newest real success invalidates all three earlier failures even when the older success has a lexically larger offset timestamp");
    record_missing_run(&db, "equal-to-success", &["native-reqwest", "curl"]);
    set_probe_time(
        &db,
        "equal-to-success",
        now - chrono::Duration::minutes(5),
        utc,
    );
    assert!(
        db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap()
            .is_empty(),
        "failure at exactly the success time is excluded"
    );
    record_missing_run(&db, "genuinely-later", &["native-reqwest", "curl"]);
    set_probe_time(
        &db,
        "genuinely-later",
        now - chrono::Duration::minutes(4),
        utc,
    );
    let remaining = db
        .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].run_id, "genuinely-later");
}

#[test]
fn an_undated_cross_run_success_cannot_reopen_failure_history() {
    let future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    for invalid_time in [
        None,
        Some(""),
        Some("!!!invalid-low"),
        Some("zzz-invalid-high"),
        Some(future.as_str()),
    ] {
        let db = HealthDatabase::in_memory().unwrap();
        for run in ["failure-1", "failure-2", "failure-3"] {
            record_missing_run(&db, run, &["native-reqwest", "curl"]);
        }
        assert_eq!(
            db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
                .unwrap()
                .len(),
            3
        );
        record_missing_run(&db, "success", &["native-reqwest"]);
        db.conn.execute("UPDATE health_attempts SET status_code = 200, body_hash = 'valid-content', content_length = 100, started_at = ?1 WHERE check_id = 'success'", params![invalid_time]).unwrap();
        assert!(db.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION).unwrap().is_empty(), "a real success of unknown time ({invalid_time:?}) must block confirmation, not be forgotten");
    }
}

#[test]
fn unrelated_or_nonreal_success_does_not_veto_valid_failure_history() {
    for excluded in [
        "other-bookmark",
        "other-config",
        "other-rule",
        "incomplete",
        "cached",
        "not-requested",
        "transport-error",
    ] {
        let db = HealthDatabase::in_memory().unwrap();
        for run in ["failure-1", "failure-2", "failure-3"] {
            record_missing_run(&db, run, &["native-reqwest", "curl"]);
        }
        let hash: String = db
            .conn
            .query_row(
                "SELECT config_hash FROM health_runs WHERE run_id = 'failure-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        record_missing_run(&db, "excluded", &["native-reqwest"]);
        db.conn.execute("UPDATE health_attempts SET status_code = 200, body_hash = 'valid-content', content_length = 100, started_at = NULL WHERE check_id = 'excluded'", []).unwrap();
        let sql = match excluded {
            "other-bookmark" => {
                "UPDATE health_checks SET bookmark_id = 'someone-else' WHERE check_id = 'excluded'"
            }
            "other-config" => {
                "UPDATE health_runs SET config_hash = 'other-config' WHERE run_id = 'excluded'"
            }
            "other-rule" => {
                "UPDATE health_runs SET rule_version = 'old-rule' WHERE run_id = 'excluded'"
            }
            "incomplete" => "UPDATE health_runs SET completed = 0 WHERE run_id = 'excluded'",
            "cached" => "UPDATE health_attempts SET cache_hit = 1 WHERE check_id = 'excluded'",
            "not-requested" => {
                "UPDATE health_attempts SET actual_request = 'no' WHERE check_id = 'excluded'"
            }
            _ => "UPDATE health_attempts SET error_kind = 'Timeout' WHERE check_id = 'excluded'",
        };
        db.conn.execute(sql, []).unwrap();
        let evidence = db
            .get_qualifying_run_evidences("bookmark", Some(&hash), CURRENT_HEALTH_RULE_VERSION)
            .unwrap();
        assert_eq!(
            evidence.len(),
            3,
            "excluded success {excluded} must not suppress another history"
        );
    }
}
