use std::{error::Error, process::Stdio, sync::Arc, time::Duration};

use axum::{
    http::{Method, StatusCode, Uri},
    response::IntoResponse,
};
use tokio::{sync::Mutex, time::Instant};

#[tokio::test]
async fn content_check_respects_site_retry_after_across_stage_boundary(
) -> Result<(), Box<dyn Error>> {
    // The entire CLI uses this local HTTP proxy; no target hostname is resolved or visited.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri| {
        let observed = Arc::clone(&observed);
        async move {
            observed
                .lock()
                .await
                .push((method, uri.path().to_owned(), Instant::now()));
            if uri.path() == "/" {
                (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "2")], "Wait before requesting this host again.").into_response()
            } else {
                axum::response::Html("<!doctype html><html><head><title>Local fixture article</title></head><body>A complete, working article served by the bounded loopback-only CLI regression fixture.</body></html>").into_response()
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(&input, "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"http://pacing.example.com/article\">fixture</A></DL><p>")?;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"));
    command
        .current_dir(temp.path())
        .args([
            "check",
            "--file",
            &input.to_string_lossy(),
            "--history-db",
            &temp.path().join("health.sqlite").to_string_lossy(),
            "--export-dir",
            &temp.path().to_string_lossy(),
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
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(15), command.output()).await;
    server.abort();
    let output = result??;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 2, "unexpected requests: {requests:?}");
    assert_eq!(
        (&requests[0].0, requests[0].1.as_str()),
        (&Method::HEAD, "/")
    );
    assert_eq!(
        (&requests[1].0, requests[1].1.as_str()),
        (&Method::GET, "/article")
    );
    assert!(
        requests[1].2.duration_since(requests[0].2) >= Duration::from_millis(1950),
        "site Retry-After: 2 was lost at the content-stage boundary: {requests:?}"
    );
    for (scope, category) in [("site", "protected_count"), ("content", "live_count")] {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            temp.path().join(scope).join("health-summary.json"),
        )?)?;
        assert_eq!(report["total_checked"], 1);
        assert_eq!(report[category], 1, "{scope}: {report}");
    }
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("check-status.json"))?)?;
    assert_eq!(status["status"], "completed");
    assert_eq!(status["phase"], "content");
    Ok(())
}
