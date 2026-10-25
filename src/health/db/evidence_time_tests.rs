//! Persisted time corruption must not manufacture qualifying death evidence.

use super::*;
use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;
use crate::health::targets::ProbeScope;

fn record_missing_run(db: &HealthDatabase, run_id: &str, backends: &[&str]) {
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
