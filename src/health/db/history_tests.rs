use super::{HealthDatabase, HistoricalCheck};
use crate::health::{
    models::{BackendObservation, HealthVerdict, ObservationSource, ProbeContext},
    targets::ProbeScope,
};
use rusqlite::{params, Connection};
use std::path::Path;

const URL: &str = "https://legacy.example.test/";

fn legacy_v3(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE health_schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
         INSERT INTO health_schema_version VALUES (3, '2026-10-29T00:00:00Z');
         CREATE TABLE health_runs (
             run_id TEXT PRIMARY KEY, started_at TEXT NOT NULL, finished_at TEXT,
             completed INTEGER NOT NULL, config_hash TEXT, rule_version TEXT
         );
         CREATE TABLE health_checks (
             check_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, bookmark_id TEXT NOT NULL,
             original_url TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT,
             run_verdict TEXT NOT NULL
         );
         CREATE TABLE health_attempts (
             attempt_id TEXT PRIMARY KEY, check_id TEXT NOT NULL, backend TEXT NOT NULL,
             backend_version TEXT NOT NULL, attempt_index INTEGER NOT NULL,
             actual_request TEXT NOT NULL, cache_hit INTEGER NOT NULL, method TEXT NOT NULL,
             status_code INTEGER, error_kind TEXT, resolver_id TEXT, dns_rcode TEXT,
             started_at TEXT, finished_at TEXT
         );
         INSERT INTO health_runs VALUES ('run-v3', '2026-10-28T12:00:00Z', NULL, 0, 'old-hash', 'old-rules');
         INSERT INTO health_checks VALUES ('check-v3', 'run-v3', 'bookmark-v3',
             'https://legacy.example.test/', '2026-10-28T12:01:00Z', NULL, 'needs_review');
         INSERT INTO health_attempts (
             attempt_id, check_id, backend, backend_version, attempt_index,
             actual_request, cache_hit, method, status_code, error_kind,
             resolver_id, dns_rcode, started_at, finished_at
         ) VALUES ('attempt-v3', 'check-v3', 'native_reqwest', '1.0', 0,
             'yes', 0, 'GET', NULL, NULL, NULL, NULL, NULL, NULL);",
    )
    .unwrap();
}

fn assert_legacy_history(checks: &[HistoricalCheck]) {
    assert_eq!(checks.len(), 1);
    let check = &checks[0];
    assert_eq!(check.run_id, "run-v3");
    assert_eq!(check.check_id, "check-v3");
    assert_eq!(check.bookmark_id, "bookmark-v3");
    assert_eq!(
        check.run_started_at.as_deref(),
        Some("2026-10-28T12:00:00Z")
    );
    assert_eq!(check.run_finished_at, None);
    assert_eq!(check.completed, Some(false));
    assert_eq!(check.config_hash.as_deref(), Some("old-hash"));
    assert_eq!(check.rule_version.as_deref(), Some("old-rules"));
    assert_eq!(check.verdict, "needs_review");
    assert_eq!(check.observations.len(), 1);
    assert_eq!(check.observations[0].attempt_id, "attempt-v3");
    assert_eq!(check.observations[0].started_at, None);
    assert_eq!(check.observations[0].finished_at, None);
    assert_eq!(check.observations[0].source, None);
}

fn schema_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT MAX(version) FROM health_schema_version",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn v3_history_is_read_without_migration_or_fabricated_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.sqlite");
    legacy_v3(&path);
    let before = std::fs::read(&path).unwrap();

    let db = HealthDatabase::open_read_only(&path).unwrap();
    assert_legacy_history(&db.get_url_history_details(URL, 10, 0).unwrap());
    assert!(db.get_url_scan_history(URL, 10, 0).unwrap().is_none());
    drop(db);

    assert_eq!(std::fs::read(&path).unwrap(), before);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(schema_version(&conn), 3);
    let source_column: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('health_attempts') WHERE name = 'source_json'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(source_column, 0);
}

#[test]
fn normal_open_migrates_v3_once_and_preserves_legacy_rows() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.sqlite");
    legacy_v3(&path);

    let db = HealthDatabase::open(&path).unwrap();
    assert_eq!(schema_version(db.raw_conn_for_test()), 5);
    assert_legacy_history(&db.get_url_history_details(URL, 10, 0).unwrap());
    assert!(db
        .get_url_scan_history(URL, 10, 0)
        .unwrap()
        .unwrap()
        .is_empty());
    drop(db);

    let db = HealthDatabase::open(&path).unwrap();
    assert_eq!(schema_version(db.raw_conn_for_test()), 5);
    assert_legacy_history(&db.get_url_history_details(URL, 10, 0).unwrap());
    let rows: i64 = db
        .raw_conn_for_test()
        .query_row(
            "SELECT COUNT(*) FROM health_attempts WHERE attempt_id = 'attempt-v3'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn failed_v4_migration_keeps_v3_version_and_rows() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("inconsistent-v3.sqlite");
    legacy_v3(&path);
    Connection::open(&path)
        .unwrap()
        .execute(
            "ALTER TABLE health_attempts ADD COLUMN source_json TEXT",
            [],
        )
        .unwrap();

    assert!(HealthDatabase::open(&path).is_err());

    let conn = Connection::open(&path).unwrap();
    assert_eq!(schema_version(&conn), 3);
    let row: (String, Option<String>) = conn
        .query_row(
            "SELECT attempt_id, source_json FROM health_attempts WHERE check_id = 'check-v3'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("attempt-v3".into(), None));
}

#[test]
fn malformed_persisted_source_is_a_history_read_error() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("corrupt-source.sqlite");
    let db = HealthDatabase::open(&path).unwrap();
    let context = ProbeContext {
        run_id: "run-source".into(),
        check_id: "check-source".into(),
        proxy_url: None,
        timeout_secs: 10,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        dns_policy_hash: "test".into(),
        allow_private_ips: false,
        history_db_path: None,
        secondary_backends: vec![],
        artifact_dir: None,
    };
    db.record_run_start(&context).unwrap();
    let observation = BackendObservation {
        source: Some(ObservationSource {
            run_id: context.run_id.clone(),
            check_id: context.check_id.clone(),
            config_hash: "test-hash".into(),
            rule_version: "test-rules".into(),
        }),
        ..BackendObservation::default()
    };
    db.record_check_result(
        &context.run_id,
        &context.check_id,
        "bookmark-source",
        URL,
        HealthVerdict::Live,
        &[observation],
    )
    .unwrap();
    db.raw_conn_for_test()
        .execute(
            "UPDATE health_attempts SET source_json = ?1 WHERE check_id = ?2",
            params!["{bad json", context.check_id],
        )
        .unwrap();
    drop(db);

    let db = HealthDatabase::open_read_only(&path).unwrap();
    assert!(db.get_url_history_details(URL, 10, 0).is_err());
}

#[test]
fn missing_v5_scan_tables_are_damage_not_legacy_unknown() {
    let db = HealthDatabase::in_memory().unwrap();
    db.raw_conn_for_test()
        .execute_batch("DROP TABLE health_scan_targets; DROP TABLE health_scans;")
        .unwrap();
    assert!(db.get_url_scan_history(URL, 10, 0).is_err());
}

#[test]
fn failed_v5_migration_rolls_back_new_tables_and_preserves_v4_rows() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("inconsistent-v4.sqlite");
    legacy_v3(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE health_attempts ADD COLUMN source_json TEXT;
         INSERT INTO health_schema_version VALUES (4, '2026-10-29T00:00:00Z');
         CREATE TABLE health_scan_targets (marker TEXT);
         INSERT INTO health_scan_targets VALUES ('preserve');",
    )
    .unwrap();
    drop(conn);

    assert!(HealthDatabase::open(&path).is_err());

    let conn = Connection::open(&path).unwrap();
    assert_eq!(schema_version(&conn), 4);
    let scan_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'health_scans'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        scan_tables, 0,
        "failed migration must not leave a partial scan schema"
    );
    let marker: String = conn
        .query_row("SELECT marker FROM health_scan_targets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(marker, "preserve");
    assert_legacy_history(
        &HealthDatabase::open_read_only(&path)
            .unwrap()
            .get_url_history_details(URL, 10, 0)
            .unwrap(),
    );
}

#[test]
fn corrupted_scan_result_json_or_identity_is_a_history_read_error() {
    let db = HealthDatabase::in_memory().unwrap();
    let context = ProbeContext {
        run_id: "unused-evidence-run".into(),
        check_id: String::new(),
        proxy_url: None,
        timeout_secs: 10,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        dns_policy_hash: "synthetic".into(),
        allow_private_ips: false,
        history_db_path: None,
        secondary_backends: vec![],
        artifact_dir: None,
    };
    let target = crate::health::models::ProbeTarget::from_url("bookmark", URL).unwrap();
    db.record_scan_start("scan", &context, &[target], false)
        .unwrap();
    db.record_scan_target("scan", "bookmark", URL, HealthVerdict::Live, &[])
        .unwrap();
    for damaged in [
        "{bad json",
        "null",
        r#"{"bookmark_id":"wrong","url":"https://legacy.example.test/","verdict":"live","observations":[]}"#,
        r#"{"bookmark_id":"bookmark","url":"https://other.example.test/","verdict":"live","observations":[]}"#,
    ] {
        db.raw_conn_for_test()
            .execute(
                "UPDATE health_scan_targets SET result_json = ?1 WHERE scan_id = 'scan'",
                [damaged],
            )
            .unwrap();
        assert!(db.get_url_scan_history(URL, 10, 0).is_err());
    }
}
