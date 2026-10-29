//! Real WAL connections with a bounded, deterministic writer interleaving.

use super::evidence_time_tests::{record_missing_run, set_probe_time};
use super::*;
use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;
use rusqlite::trace::{TraceEvent, TraceEventCodes};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn evidence_read_cannot_mix_a_precommit_success_cutoff_with_postcommit_failures() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("health.sqlite");
    let mut reader = HealthDatabase::open(&path).unwrap();
    let now = Utc::now();
    let utc = chrono::FixedOffset::east_opt(0).unwrap();
    for (run, minutes) in [("failure", 10), ("success", 5)] {
        record_missing_run(&reader, run, &["native-reqwest", "curl"]);
        set_probe_time(&reader, run, now - chrono::Duration::minutes(minutes), utc);
    }
    reader.conn.execute("UPDATE health_attempts SET status_code = 200, body_hash = 'valid-content', content_length = 100 WHERE check_id = 'success'", []).unwrap();
    reader
        .conn
        .execute("UPDATE health_runs SET completed = 0", [])
        .unwrap();
    assert!(reader
        .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
        .unwrap()
        .is_empty());

    let writer = Connection::open(&path).unwrap();
    writer.busy_timeout(Duration::from_secs(2)).unwrap();
    let (trigger, begin_write) = mpsc::sync_channel(1);
    let (committed, commit_result) = mpsc::sync_channel(1);
    let (report, observed) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = begin_write.recv_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())
            .and_then(|()| writer.execute("UPDATE health_runs SET completed = 1 WHERE run_id IN ('failure', 'success')", []).map_err(|error| error.to_string()));
        // Both the thread result and callback acknowledgement are asserted outside callbacks.
        let _ = committed.send(result.clone());
        result
    });
    let mut fired = false;
    reader
        .conn
        .trace_v2(
            TraceEventCodes::SQLITE_TRACE_PROFILE,
            Some(move |event: TraceEvent<'_>| {
                let TraceEvent::Profile(statement, _) = event else {
                    return;
                };
                let sql = statement.sql();
                if fired || !sql.trim_start().starts_with("SELECT") || !sql.contains("health_") {
                    return;
                }
                fired = true;
                let outcome = trigger
                    .try_send(())
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        commit_result
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|error| error.to_string())
                    })
                    .and_then(|result| result);
                // rusqlite catches callback panics, so assertions must stay outside this callback.
                // A missing report also fails the test below; it cannot manufacture a pass.
                let _ = report.try_send(outcome);
            }),
        )
        .unwrap();
    let result = reader.get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION);
    reader
        .conn
        .trace_v2(TraceEventCodes::empty(), None::<fn(TraceEvent<'_>)>)
        .unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap(),
        2,
        "writer must actually commit both runs"
    );
    assert_eq!(
        observed
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap(),
        2,
        "the bounded interleaving must have executed"
    );
    let evidence = result.unwrap();
    assert!(evidence.is_empty(), "mixed snapshot returned {evidence:?}: precommit has no completed evidence; postcommit has a later success; neither permits the older failure");
    assert!(
        reader.conn.is_autocommit(),
        "the read must release its owned snapshot"
    );
    assert!(
        reader
            .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap()
            .is_empty(),
        "a fresh read must see the committed protecting success"
    );
    reader
        .conn
        .execute(
            "UPDATE health_runs SET completed = 0 WHERE run_id = 'success'",
            [],
        )
        .unwrap();
    let failures = reader
        .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
        .unwrap();
    assert_eq!(
        failures.len(),
        1,
        "a later read must see committed changes, not retain a stale snapshot"
    );
    assert_eq!(failures[0].run_id, "failure");
}

#[test]
fn evidence_read_preserves_a_callers_pending_transaction() {
    let reader = HealthDatabase::in_memory().unwrap();
    record_missing_run(&reader, "failure", &["native-reqwest", "curl"]);
    reader.conn.execute_batch("CREATE TABLE caller_marker (value INTEGER); BEGIN; INSERT INTO caller_marker VALUES (1);").unwrap();
    assert!(!reader.conn.is_autocommit());
    assert_eq!(
        reader
            .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .unwrap()
            .len(),
        1
    );
    assert!(
        !reader.conn.is_autocommit(),
        "the evidence read must not finish the caller's transaction"
    );
    assert_eq!(
        reader
            .conn
            .query_row("SELECT value FROM caller_marker", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    reader.conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        reader
            .conn
            .query_row("SELECT COUNT(*) FROM caller_marker", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "the marker must remain rollbackable, not silently committed"
    );
}

#[test]
fn evidence_read_errors_release_only_owned_transactions() {
    for outer in [false, true] {
        let reader = HealthDatabase::in_memory().unwrap();
        reader
            .conn
            .execute_batch("CREATE TABLE caller_marker (value INTEGER)")
            .unwrap();
        if outer {
            reader
                .conn
                .execute_batch("BEGIN; INSERT INTO caller_marker VALUES (1)")
                .unwrap();
        }
        // A synthetic schema fault makes the first evidence query fail.
        reader
            .conn
            .execute_batch("DROP TABLE health_attempts")
            .unwrap();
        assert!(reader
            .get_qualifying_run_evidences("bookmark", None, CURRENT_HEALTH_RULE_VERSION)
            .is_err());
        assert_eq!(reader.conn.is_autocommit(), !outer);
        if outer {
            assert_eq!(
                reader
                    .conn
                    .query_row("SELECT value FROM caller_marker", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            reader.conn.execute_batch("ROLLBACK").unwrap();
            assert_eq!(
                reader
                    .conn
                    .query_row("SELECT COUNT(*) FROM caller_marker", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}
