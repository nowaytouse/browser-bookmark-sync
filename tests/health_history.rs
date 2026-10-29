use std::{
    error::Error,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use browser_bookmark_sync::health::{
    db::HealthDatabase,
    models::{BackendObservation, HealthVerdict, ObservationSource, ProbeContext},
    targets::ProbeScope,
};
use serde_json::Value;

const CLI: &str = env!("CARGO_BIN_EXE_browser-bookmark-sync");
const URL: &str = "https://history.example.test/article";

fn context(run_id: &str, check_id: &str) -> ProbeContext {
    ProbeContext {
        run_id: run_id.into(),
        check_id: check_id.into(),
        proxy_url: None,
        timeout_secs: 10,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        dns_policy_hash: "history-test-policy".into(),
        allow_private_ips: false,
        history_db_path: None,
        secondary_backends: vec![],
        artifact_dir: None,
    }
}

fn observation(
    backend: &str,
    source: Option<ObservationSource>,
) -> Result<BackendObservation, chrono::ParseError> {
    Ok(BackendObservation {
        backend: backend.into(),
        backend_version: "test-1".into(),
        actual_request: "yes".into(),
        cache_hit: false,
        request_method: "GET".into(),
        initial_url: URL.into(),
        started_at: "2026-10-29T01:02:03Z".parse()?,
        finished_at: "2026-10-29T01:02:04Z".parse()?,
        source,
        ..BackendObservation::default()
    })
}

fn run_cli(db: &std::path::Path, args: &[&str]) -> Result<std::process::Output, Box<dyn Error>> {
    let mut child = Command::new(CLI)
        .args(["health", "history", "--db"])
        .arg(db)
        .args(["--url", URL])
        .args(args)
        .current_dir(db.parent().ok_or("missing fixture directory")?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err("history CLI exceeded fixture deadline".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(child.wait_with_output()?)
}

fn seed_history(path: &std::path::Path) -> Result<ObservationSource, Box<dyn Error>> {
    let db = HealthDatabase::open(path)?;

    let first = context("run-original", "check-original");
    db.record_run_start(&first)?;
    let original_source = ObservationSource {
        run_id: first.run_id.clone(),
        check_id: first.check_id.clone(),
        config_hash: browser_bookmark_sync::health::models::ConfigHashInput::from_probe_context(
            &first,
        )
        .stable_hash()?,
        rule_version: browser_bookmark_sync::health::state_machine::CURRENT_HEALTH_RULE_VERSION
            .into(),
    };
    db.record_check_result(
        &first.run_id,
        &first.check_id,
        "bookmark-1",
        URL,
        HealthVerdict::Live,
        &[
            observation("native_reqwest", Some(original_source.clone()))?,
            observation("native_dns", None)?,
        ],
    )?;
    db.record_run_finish(&first.run_id)?;

    // This represents a later cached/reused result: source identity remains the original probe.
    let second = context("run-cache-use", "check-cache-use");
    db.record_run_start(&second)?;
    let mut reused = observation("native_dns", Some(original_source.clone()))?;
    reused.cache_hit = true;
    reused.actual_request = "no".into();
    db.record_check_observations(
        &second.run_id,
        &second.check_id,
        "bookmark-1",
        URL,
        HealthVerdict::Live,
        &[reused],
    )?;
    db.record_run_finish(&second.run_id)?;

    // An incomplete run remains visible and must not be relabeled as completed.
    let interrupted = context("run-interrupted", "check-interrupted");
    db.record_run_start(&interrupted)?;
    db.record_check_observations(
        &interrupted.run_id,
        &interrupted.check_id,
        "bookmark-1",
        URL,
        HealthVerdict::NeedsReview,
        &[observation("native_reqwest", None)?],
    )?;
    drop(db);
    Ok(original_source)
}

#[test]
fn history_json_preserves_run_attempt_and_original_source_after_reopen(
) -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("health.sqlite");
    let expected_source = seed_history(&db)?;
    let before = std::fs::read(&db)?;

    let page = run_cli(&db, &["--json", "--limit", "1", "--offset", "0"])?;
    assert!(
        page.status.success(),
        "{}",
        String::from_utf8_lossy(&page.stderr)
    );
    let page: Value = serde_json::from_slice(&page.stdout)?;
    assert_eq!(page["schema_version"], "health-history-v1");
    assert_eq!(page["url"], URL);
    assert_eq!(
        (page["limit"].as_u64(), page["offset"].as_u64()),
        (Some(1), Some(0))
    );
    assert_eq!(
        page["checks"]
            .as_array()
            .ok_or("checks array missing")?
            .len(),
        1
    );
    let check = &page["checks"][0];
    assert_eq!(check["run_id"], "run-interrupted");
    assert_eq!(check["check_id"], "check-interrupted");
    assert_eq!(check["completed"], false);
    assert_eq!(
        check["observations"]
            .as_array()
            .ok_or("observations array missing")?
            .len(),
        1
    );
    assert!(check["observations"][0]["source"].is_null());

    let page = run_cli(&db, &["--json", "--limit", "1", "--offset", "1"])?;
    assert!(
        page.status.success(),
        "{}",
        String::from_utf8_lossy(&page.stderr)
    );
    let page: Value = serde_json::from_slice(&page.stdout)?;
    assert_eq!(
        page["checks"]
            .as_array()
            .ok_or("checks array missing")?
            .len(),
        1
    );
    let check = &page["checks"][0];
    assert_eq!(check["run_id"], "run-cache-use");
    assert_eq!(check["completed"], true);
    assert_eq!(
        check["observations"]
            .as_array()
            .ok_or("observations array missing")?
            .len(),
        1
    );
    let attempt = &check["observations"][0];
    assert_eq!(attempt["cache_hit"], true);
    assert_eq!(attempt["actual_request"], "no");
    assert_eq!(attempt["source"]["run_id"], "run-original");
    assert_eq!(attempt["source"]["check_id"], "check-original");
    assert_eq!(
        attempt["source"]["config_hash"],
        expected_source.config_hash
    );
    assert_eq!(
        attempt["source"]["rule_version"],
        expected_source.rule_version
    );

    let page = run_cli(&db, &["--json", "--limit", "1", "--offset", "2"])?;
    assert!(
        page.status.success(),
        "{}",
        String::from_utf8_lossy(&page.stderr)
    );
    let page: Value = serde_json::from_slice(&page.stdout)?;
    let check = &page["checks"][0];
    assert_eq!(check["run_id"], "run-original");
    assert_eq!(
        check["observations"]
            .as_array()
            .ok_or("observations array missing")?
            .len(),
        2
    );
    assert_eq!(
        check["observations"][0]["source"]["config_hash"],
        expected_source.config_hash
    );
    assert_eq!(
        check["observations"][0]["started_at"],
        "2026-10-29T01:02:03+00:00"
    );
    assert_eq!(
        check["observations"][0]["finished_at"],
        "2026-10-29T01:02:04+00:00"
    );
    assert!(check["observations"][1]["source"].is_null());

    assert_eq!(
        std::fs::read(&db)?,
        before,
        "history read must not mutate DB bytes"
    );
    Ok(())
}

#[test]
fn history_missing_database_fails_without_creating_it() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("missing.sqlite");
    let output = run_cli(&db, &[])?;
    assert!(
        !output.status.success(),
        "missing history DB must be an error"
    );
    assert!(
        !db.exists(),
        "history must not create a requested missing DB"
    );
    Ok(())
}
