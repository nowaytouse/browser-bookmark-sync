#[cfg(unix)]
#[test]
fn sigterm_marks_check_aborted_and_logs_reason() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        fs,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    let status_path = temp.path().join("check-status.json");
    let log_path = temp.path().join("browser-sync.log");
    let db_path = temp.path().join("bookmark-health.sqlite");
    // Keep the fixture port reserved without serving a response: the signal test
    // must never resolve or visit its synthetic target on the public network.
    let proxy_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let proxy = format!("http://{}", proxy_listener.local_addr()?);

    let mut html = String::from("<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p>\n");
    for index in 0..100 {
        html.push_str(&format!(
            "<DT><A HREF=\"https://example.com/signal-test-{index}\">test</A>\n"
        ));
    }
    html.push_str("</DL><p>\n");
    fs::write(&input, html)?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"))
        .args([
            "--log-file",
            &log_path.to_string_lossy(),
            "check",
            "--file",
            &input.to_string_lossy(),
            "--history-db",
            &db_path.to_string_lossy(),
            "--export-dir",
            &temp.path().to_string_lossy(),
            "--scope",
            "content",
            "--proxy",
            &proxy,
            "--timeout",
            "10",
            "--concurrency",
            "1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(10);
    while !status_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    if !status_path.exists() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("check process did not create status file".into());
    }

    let signal_status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    if !signal_status.success() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("kill command failed".into());
    }
    let exit_status = child.wait()?;
    assert!(
        !exit_status.success(),
        "SIGTERM should stop the check process"
    );

    let status: serde_json::Value = serde_json::from_str(&fs::read_to_string(&status_path)?)?;
    assert_eq!(status["status"], "aborted");

    let log = fs::read_to_string(&log_path)?;
    assert!(log.contains("process_terminated"));
    assert!(log.contains("check_status_aborted"));
    assert!(log.contains("SIGTERM"));
    Ok(())
}

#[tokio::test]
async fn check_persists_real_site_and_content_progress() -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, process::Stdio, time::Duration};

    // A loopback-only proxy fixture: never visit a real bookmarked site.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let app = axum::Router::new().fallback(|| async {
        axum::response::Html("<!doctype html><html><head><title>Healthy fixture</title></head><body>A working article for the local progress regression test.</body></html>")
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    let fixture_db = temp.path().join("fixture-health.sqlite");
    fs::write(&input, "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"http://progress.example.com/article\">test</A></DL><p>")?;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"));
    command
        .current_dir(temp.path())
        .args([
            "check",
            "--file",
            &input.to_string_lossy(),
            "--export-dir",
            &temp.path().to_string_lossy(),
            "--history-db",
            &fixture_db.to_string_lossy(),
            "--scope",
            "content",
            "--proxy",
            &proxy,
            "--timeout",
            "1",
            "--concurrency",
            "1",
            "--no-trusted-site-skip",
        ])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let result = tokio::time::timeout(Duration::from_secs(30), command.output()).await;
    server.abort();
    let output = result??;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("check-status.json"))?)?;
    assert_eq!(status["status"], "completed");
    assert_eq!(status["phase"], "content");
    assert_eq!(status["completed_targets"], 1);
    assert_eq!(status["total_targets"], 1);
    for scope in ["site", "content"] {
        let report: serde_json::Value = serde_json::from_slice(&fs::read(
            temp.path().join(scope).join("health-summary.json"),
        )?)?;
        assert_eq!(report["total_checked"], 1);
        assert_eq!(report["live_count"], 1, "{scope} report: {report}");
    }
    Ok(())
}
