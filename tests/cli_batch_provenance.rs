use std::{error::Error, process::Stdio, sync::Arc, time::Duration};

use serde_json::Value;
use tokio::sync::Mutex;

struct AbortServer(tokio::task::AbortHandle);

impl Drop for AbortServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn read_json(path: impl AsRef<std::path::Path>) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

async fn run_batch(
    root: &std::path::Path,
    proxy: &str,
    batch: &str,
    limit: usize,
    expected_partial: bool,
    independent: bool,
) -> Result<Value, Box<dyn Error>> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"));
    let mut reports = root.join(batch);
    let mut db = root.join("health.sqlite");
    let mut input = root.join("input.html");
    command.current_dir(root).arg("check");
    if independent {
        std::fs::create_dir(&reports)?;
        db = reports.join("bookmark-health.sqlite");
        std::fs::copy(&input, reports.join("input.html"))?;
        input = reports.join("input.html");
        reports = reports.join("health");
        command.arg("--green-memory").arg(root.join("green.sqlite"));
    }
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        command
            .arg("--file")
            .arg(input)
            .arg("--history-db")
            .arg(db)
            .arg("--export-dir")
            .arg(&reports)
            .arg("--limit")
            .arg(limit.to_string())
            .args([
                "--scope",
                "content",
                "--proxy",
                proxy,
                "--timeout",
                "2",
                "--concurrency",
                "1",
                "--no-trusted-site-skip",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let directory = reports.join("content");
    let summary = read_json(directory.join("health-summary.json"))?;
    let provenance = read_json(directory.join("health-provenance.json"))?;
    assert_eq!(summary["scan_id"], provenance["scan_id"]);
    assert_eq!(summary["run_id"], provenance["scan_id"]);
    assert_eq!(summary["provenance"], provenance["counts"]);
    assert_eq!(summary["partial"], expected_partial);
    assert_eq!(
        provenance["accounting_scope"],
        "final_reported_evidence_not_total_traffic"
    );
    assert_eq!(
        read_json(reports.join("check-status.json"))?["status"],
        "completed"
    );
    Ok(provenance)
}

#[tokio::test]
async fn successive_cli_batches_preserve_sources_and_only_probe_new_targets(
) -> Result<(), Box<dyn Error>> {
    // A local non-forwarding proxy answers every request; target names are never resolved.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().fallback(move |method: axum::http::Method, uri: axum::http::Uri| {
            let observed = Arc::clone(&observed);
            async move {
                observed.lock().await.push((method.to_string(), uri.path().to_string()));
                axum::response::Html("<!doctype html><html><head><title>Healthy synthetic article</title></head><body>This complete article is provided solely by the local test proxy. No remote site, DNS provider, browser Profile or personal bookmark is used.</body></html>")
            }
        })).await
    });
    let _server = AbortServer(server.abort_handle());
    let root = tempfile::tempdir()?;
    let initial_input = "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"http://batch.example.com/one\">one</A></DL><p>";
    std::fs::write(root.path().join("input.html"), initial_input)?;

    let first = run_batch(root.path(), &proxy, "first", 100, false, false).await?;
    assert_eq!(first["counts"]["fresh_probe_targets"], 1);
    assert_eq!(first["counts"]["reused_only_targets"], 0);
    assert_eq!(
        *requests.lock().await,
        [
            ("HEAD".to_string(), "/".to_string()),
            ("GET".to_string(), "/one".to_string())
        ]
    );
    let original = &first["targets"][0]["observations"][0];
    assert_eq!(original["actual_request"], "yes");
    assert_eq!(original["source"]["run_id"], original["run_id"]);
    assert_eq!(original["source"]["check_id"], original["check_id"]);

    let second = run_batch(root.path(), &proxy, "second", 1, false, false).await?;
    assert_ne!(first["scan_id"], second["scan_id"]);
    assert_eq!(second["counts"]["fresh_probe_targets"], 0);
    assert_eq!(second["counts"]["reused_only_targets"], 1);
    assert_eq!(
        *requests.lock().await,
        [
            ("HEAD".to_string(), "/".to_string()),
            ("GET".to_string(), "/one".to_string())
        ],
        "unchanged input must not add traffic"
    );
    let reused = &second["targets"][0]["observations"][0];
    assert_eq!(reused["actual_request"], "no");
    assert_eq!(reused["cache_hit"], true);
    assert_eq!(reused["source"], original["source"]);
    assert_eq!(reused["started_at"], original["started_at"]);
    assert_eq!(reused["finished_at"], original["finished_at"]);

    std::fs::write(root.path().join("input.html"), "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>\n<DT><A HREF=\"http://batch.example.com/one\">one</A>\n<DT><A HREF=\"http://batch.example.com/two\">two</A>\n</DL><p>\n")?;
    let third = run_batch(root.path(), &proxy, "third", 100, false, false).await?;
    assert_ne!(second["scan_id"], third["scan_id"]);
    assert_eq!(third["total_targets"], 2);
    assert_eq!(third["counts"]["fresh_probe_targets"], 1);
    assert_eq!(third["counts"]["reused_only_targets"], 1);
    assert_eq!(
        *requests.lock().await,
        [
            ("HEAD".to_string(), "/".to_string()),
            ("GET".to_string(), "/one".to_string()),
            ("GET".to_string(), "/two".to_string())
        ]
    );
    let targets = third["targets"]
        .as_array()
        .ok_or("missing target records")?;
    let old = targets
        .iter()
        .find(|target| target["url"] == "http://batch.example.com/one")
        .ok_or("missing reused target")?;
    let new = targets
        .iter()
        .find(|target| target["url"] == "http://batch.example.com/two")
        .ok_or("missing fresh target")?;
    assert_eq!(old["observations"][0]["source"], original["source"]);
    assert_eq!(new["observations"][0]["actual_request"], "yes");
    assert_ne!(
        new["observations"][0]["source"]["run_id"],
        original["source"]["run_id"]
    );

    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"))
            .current_dir(root.path())
            .args(["health", "history", "--scans", "--json", "--db"])
            .arg(root.path().join("health.sqlite"))
            .args(["--url", "http://batch.example.com/one"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let history: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(history["schema_version"], "health-scan-history-v1");
    assert_eq!(history["recording_available"], true);
    let scans = history["scans"].as_array().ok_or("missing scan history")?;
    assert_eq!(
        scans.len(),
        3,
        "reuse-only batches must remain discoverable"
    );
    for (stored, report) in scans.iter().zip([&third, &second, &first]) {
        assert_eq!(stored["scan_id"], report["scan_id"]);
        assert_eq!(stored["scope"], "content");
        assert_eq!(stored["completed"], true);
        assert_eq!(stored["partial"], false);
        let expected = report["targets"]
            .as_array()
            .ok_or("missing report targets")?
            .iter()
            .find(|target| target["url"] == "http://batch.example.com/one")
            .ok_or("missing original URL")?;
        assert_eq!(&stored["result"], expected);
    }
    let db = browser_bookmark_sync::health::db::HealthDatabase::open_read_only(
        root.path().join("health.sqlite"),
    )?;
    assert_eq!(
        db.get_url_history_details("http://batch.example.com/one", 50, 0)?
            .len(),
        1,
        "recording membership must not manufacture independent checks"
    );
    let limited = run_batch(root.path(), &proxy, "limited", 1, true, false).await?;
    assert_eq!(limited["total_targets"], 1);
    assert_eq!(
        requests.lock().await.len(),
        3,
        "limited reuse must not add traffic"
    );
    let limited_url = limited["targets"][0]["url"]
        .as_str()
        .ok_or("missing limited URL")?;
    let page = db
        .get_url_scan_history(limited_url, 1, 0)?
        .ok_or("scan recording unavailable")?;
    assert_eq!(page[0].scan_id, limited["scan_id"]);
    assert!(page[0].partial);
    assert_eq!(page[0].planned_targets, 1);

    // The GUI deliberately uses a different batch DB every time. Count actual
    // proxy traffic, including the site root; report "reuse" alone is not proof.
    requests.lock().await.clear();
    let cold = run_batch(root.path(), &proxy, "gui-a", 0, false, true).await?;
    assert_eq!(cold["counts"]["fresh_probe_targets"], 2);
    assert_eq!(requests.lock().await.len(), 3);
    let warm = run_batch(root.path(), &proxy, "gui-b", 0, false, true).await?;
    assert_eq!(warm["counts"]["reused_only_targets"], 2);
    assert_eq!(
        requests.lock().await.len(),
        3,
        "green site and both green pages must add zero requests"
    );
    let warm_dir = root.path().join("gui-b");
    let site = read_json(warm_dir.join("health/site/health-provenance.json"))?;
    assert_eq!(site["counts"]["reused_only_targets"], 1);
    #[cfg(all(feature = "native-gui", target_os = "macos"))]
    {
        let sha = std::process::Command::new("/usr/bin/shasum")
            .args(["-a", "256"])
            .arg(warm_dir.join("input.html"))
            .output()?;
        assert!(sha.status.success());
        let hash = std::str::from_utf8(&sha.stdout)?
            .split_whitespace()
            .next()
            .ok_or("missing hash")?;
        std::fs::write(
            warm_dir.join("run-manifest.json"),
            serde_json::to_vec(&serde_json::json!({"input_sha256":hash}))?,
        )?;
        std::fs::write(
            warm_dir.join("process-exit.json"),
            br#"{"process_exit_code":0}"#,
        )?;
        browser_bookmark_sync::native_scan::ScanHandle {
            directory: warm_dir,
        }
        .verify_complete()?;
    }
    Ok(())
}
