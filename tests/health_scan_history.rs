use std::{error::Error, io};

use browser_bookmark_sync::health::{
    db::HealthDatabase,
    models::{BackendObservation, HealthVerdict, ProbeContext, ProbeTarget},
    targets::ProbeScope,
};

const URL: &str = "https://example.com/article";

fn context(id: &str) -> ProbeContext {
    ProbeContext {
        run_id: format!("run-{id}"),
        check_id: format!("check-{id}"),
        proxy_url: None,
        timeout_secs: 10,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        dns_policy_hash: "synthetic-test-policy".into(),
        allow_private_ips: false,
        history_db_path: None,
        secondary_backends: Vec::new(),
        artifact_dir: None,
    }
}

fn target(bookmark_id: &str, url: &str) -> Result<ProbeTarget, Box<dyn Error>> {
    let target = ProbeTarget::from_url(bookmark_id, url).map_err(io::Error::other)?;
    Ok(target)
}

fn only_history_row(
    db: &HealthDatabase,
    url: &str,
) -> Result<browser_bookmark_sync::health::db::HistoricalScanTarget, Box<dyn Error>> {
    let rows = db
        .get_url_scan_history(url, 10, 0)?
        .ok_or_else(|| io::Error::other("scan-history schema was not available"))?;
    if rows.len() != 1 {
        return Err(
            io::Error::other(format!("expected one history row, got {}", rows.len())).into(),
        );
    }
    rows.into_iter()
        .next()
        .ok_or_else(|| io::Error::other("history row disappeared").into())
}

#[test]
fn pending_membership_survives_reopen_and_rejects_early_finish_and_overwrites(
) -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("health.sqlite");
    let target = target("bookmark-1", URL)?;
    let db = HealthDatabase::open(&path)?;
    db.record_scan_start("scan-pending", &context("pending"), &[target], true)?;
    drop(db);

    let db = HealthDatabase::open(&path)?;
    let pending = only_history_row(&db, URL)?;
    assert_eq!(pending.scan_id, "scan-pending");
    assert!(pending.partial);
    assert_eq!(pending.planned_targets, 1);
    assert!(!pending.completed);
    assert!(pending.finished_at.is_none());
    assert!(pending.result.is_none());
    assert!(db.record_scan_finish("scan-pending").is_err());

    assert!(db
        .record_scan_target(
            "scan-pending",
            "wrong-bookmark",
            URL,
            HealthVerdict::Live,
            &[],
        )
        .is_err());
    assert!(db
        .record_scan_target(
            "scan-pending",
            "bookmark-1",
            "https://other.example.com/",
            HealthVerdict::Live,
            &[],
        )
        .is_err());

    db.record_scan_target("scan-pending", "bookmark-1", URL, HealthVerdict::Live, &[])?;
    assert!(db
        .record_scan_target(
            "scan-pending",
            "bookmark-1",
            URL,
            HealthVerdict::NeedsReview,
            &[],
        )
        .is_err());
    db.record_scan_finish("scan-pending")?;

    let completed = only_history_row(&db, URL)?;
    assert!(completed.completed);
    assert!(completed.finished_at.is_some());
    let result = completed
        .result
        .as_ref()
        .ok_or_else(|| io::Error::other("completed target has no result"))?;
    assert_eq!(result["verdict"], "live");
    assert_eq!(result["observations"].as_array().map(Vec::len), Some(0));

    assert!(db
        .record_scan_target(
            "scan-pending",
            "bookmark-1",
            URL,
            HealthVerdict::NeedsReview,
            &[BackendObservation::default()],
        )
        .is_err());
    assert!(db.record_scan_finish("scan-pending").is_err());
    assert_eq!(only_history_row(&db, URL)?.result, completed.result);
    Ok(())
}

#[test]
fn pagination_selects_scans_and_keeps_same_url_bookmark_ids_together() -> Result<(), Box<dyn Error>>
{
    let temp = tempfile::tempdir()?;
    let db = HealthDatabase::open(temp.path().join("health.sqlite"))?;
    let older_targets = [target("bookmark-a", URL)?, target("bookmark-b", URL)?];
    db.record_scan_start("scan-older", &context("older"), &older_targets, false)?;
    for item in &older_targets {
        db.record_scan_target(
            "scan-older",
            &item.bookmark_id,
            URL,
            HealthVerdict::Live,
            &[],
        )?;
    }
    db.record_scan_finish("scan-older")?;

    let newer_target = target("bookmark-c", URL)?;
    db.record_scan_start("scan-newer", &context("newer"), &[newer_target], false)?;
    db.record_scan_target("scan-newer", "bookmark-c", URL, HealthVerdict::Live, &[])?;
    db.record_scan_finish("scan-newer")?;

    let latest = db
        .get_url_scan_history(URL, 1, 0)?
        .ok_or_else(|| io::Error::other("scan-history schema was not available"))?;
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].scan_id, "scan-newer");
    assert_eq!(latest[0].bookmark_id, "bookmark-c");

    let previous = db
        .get_url_scan_history(URL, 1, 1)?
        .ok_or_else(|| io::Error::other("scan-history schema was not available"))?;
    assert_eq!(previous.len(), 2);
    assert!(previous.iter().all(|row| row.scan_id == "scan-older"));
    assert_eq!(
        previous
            .iter()
            .map(|row| row.bookmark_id.as_str())
            .collect::<Vec<_>>(),
        vec!["bookmark-a", "bookmark-b"]
    );
    assert!(previous.iter().all(|row| row.url == URL && row.completed));
    Ok(())
}

#[test]
fn scan_history_rejects_out_of_range_limits() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = HealthDatabase::open(temp.path().join("health.sqlite"))?;
    assert!(db.get_url_scan_history(URL, 0, 0).is_err());
    assert!(db.get_url_scan_history(URL, 501, 0).is_err());
    Ok(())
}
