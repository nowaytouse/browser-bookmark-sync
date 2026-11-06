use std::{
    error::Error,
    process::{Output, Stdio},
    sync::Arc,
    time::Duration,
};

use axum::{
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
};
use tokio::{sync::Mutex, time::Instant};

const HEALTHY_HTML: &str = "<!doctype html><html><head><title>Local fixture article</title></head><body>A complete, working article served by the bounded loopback-only CLI regression fixture.</body></html>";

async fn run_content_check(
    proxy: &str,
    slow: bool,
) -> Result<(tempfile::TempDir, Output), Box<dyn Error>> {
    run_content_check_with_timeout(proxy, slow, if slow { "10" } else { "1" }).await
}

async fn run_content_check_with_timeout(
    proxy: &str,
    slow: bool,
    request_timeout: &str,
) -> Result<(tempfile::TempDir, Output), Box<dyn Error>> {
    run_content_check_with_input(
        proxy,
        slow,
        request_timeout,
        "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"http://pacing.example.com/article\">fixture</A></DL><p>",
        "1",
    )
    .await
}

async fn run_content_check_with_input(
    proxy: &str,
    slow: bool,
    request_timeout: &str,
    html: &str,
    concurrency: &str,
) -> Result<(tempfile::TempDir, Output), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(&input, html)?;
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
            proxy,
            "--timeout",
            request_timeout,
            "--concurrency",
            concurrency,
            "--no-trusted-site-skip",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if slow {
        command.arg("--slow");
    }
    let output = tokio::time::timeout(Duration::from_secs(15), command.output()).await??;
    Ok((temp, output))
}

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
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "2")],
                    "Wait before requesting this host again.",
                )
                    .into_response()
            } else {
                axum::response::Html(HEALTHY_HTML).into_response()
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = run_content_check(&proxy, false).await;
    server.abort();
    let (temp, output) = result?;
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

#[tokio::test]
async fn different_origins_share_redirect_destination_pacing() -> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri| {
        let observed = Arc::clone(&observed);
        async move {
            let article = uri.path() == "/article";
            observed.lock().await.push((method, uri, Instant::now()));
            if article {
                (
                    StatusCode::FOUND,
                    [("location", "http://shared.example.com/landing")],
                )
                    .into_response()
            } else {
                axum::response::Html(HEALTHY_HTML).into_response()
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = run_content_check_with_input(
        &proxy,
        true,
        "10",
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>\n<DT><A HREF=\"http://alpha.example.org/article\">alpha</A>\n<DT><A HREF=\"http://beta.example.net/article\">beta</A>\n</DL><p>\n",
        "2",
    )
    .await;
    server.abort();
    let (temp, output) = result?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 6, "unexpected requests: {requests:?}");
    assert_eq!(
        requests
            .iter()
            .filter(|(method, uri, _)| *method == Method::HEAD && uri.path() == "/")
            .count(),
        2,
        "both original sites must actually be preflighted: {requests:?}"
    );
    let article_hosts: std::collections::BTreeSet<_> = requests
        .iter()
        .filter(|(method, uri, _)| *method == Method::GET && uri.path() == "/article")
        .filter_map(|(_, uri, _)| uri.host())
        .collect();
    assert_eq!(
        article_hosts,
        std::collections::BTreeSet::from(["alpha.example.org", "beta.example.net"]),
        "both distinct original domains must actually be requested"
    );
    let landings: Vec<_> = requests
        .iter()
        .filter(|(method, uri, _)| {
            *method == Method::GET
                && uri.host() == Some("shared.example.com")
                && uri.path() == "/landing"
        })
        .collect();
    assert_eq!(
        landings.len(),
        2,
        "both redirects must actually reach the shared host: {requests:?}"
    );
    assert!(
        landings[1].2.duration_since(landings[0].2) >= Duration::from_millis(1950),
        "different original domains bypassed the shared destination's pacing: {requests:?}"
    );
    for scope in ["site", "content"] {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            temp.path().join(scope).join("health-summary.json"),
        )?)?;
        assert_eq!(report["total_checked"], 2);
        assert_eq!(report["live_count"], 2, "{scope}: {report}");
    }
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("check-status.json"))?)?;
    assert_eq!(status["status"], "completed");
    assert_eq!(status["phase"], "content");
    assert_eq!(status["completed_targets"], 2);
    assert_eq!(status["total_targets"], 2);
    Ok(())
}

#[tokio::test]
async fn slow_content_check_paces_every_same_host_redirect() -> Result<(), Box<dyn Error>> {
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
            match uri.path() {
                "/article" => (StatusCode::FOUND, [("location", "/middle")]).into_response(),
                "/middle" => (StatusCode::SEE_OTHER, [("location", "/final")]).into_response(),
                _ => axum::response::Html(HEALTHY_HTML).into_response(),
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = run_content_check(&proxy, true).await;
    server.abort();
    let (temp, output) = result?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 4, "unexpected requests: {requests:?}");
    for (index, (method, path)) in [
        (Method::HEAD, "/"),
        (Method::GET, "/article"),
        (Method::GET, "/middle"),
        (Method::GET, "/final"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            (&requests[index].0, requests[index].1.as_str()),
            (&method, path)
        );
    }
    for pair in requests.windows(2) {
        assert!(
            pair[1].2.duration_since(pair[0].2) >= Duration::from_millis(1950),
            "a same-host phase or redirect request bypassed slow-mode pacing: {requests:?}"
        );
    }
    for scope in ["site", "content"] {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            temp.path().join(scope).join("health-summary.json"),
        )?)?;
        assert_eq!(report["total_checked"], 1);
        assert_eq!(report["live_count"], 1, "{scope}: {report}");
    }
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("check-status.json"))?)?;
    assert_eq!(status["status"], "completed");
    assert_eq!(status["phase"], "content");
    Ok(())
}

#[tokio::test]
async fn cross_host_redirect_respects_retry_after_without_adding_location_credentials(
) -> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri, headers: HeaderMap| {
        let observed = Arc::clone(&observed);
        async move {
            let article = uri.path() == "/article";
            observed
                .lock()
                .await
                .push((method, uri, Instant::now(), headers));
            if article {
                (
                    StatusCode::FOUND,
                    [
                        (
                            "location",
                            "http://fixture-user:fixture-password@destination.example.net/landing",
                        ),
                        ("retry-after", "2"),
                    ],
                )
                    .into_response()
            } else {
                axum::response::Html(HEALTHY_HTML).into_response()
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    // Keep standard-mode pacing, but allow the chain to wait the server's two seconds.
    let result = run_content_check_with_input(
        &proxy,
        false,
        "10",
        "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"http://user:pass@pacing.example.com/article\">synthetic credentials</A></DL><p>",
        "1",
    ).await;
    server.abort();
    let (temp, output) = result?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 3, "unexpected requests: {requests:?}");
    assert_eq!((&requests[0].0, requests[0].1.path()), (&Method::HEAD, "/"));
    assert_eq!(
        (&requests[1].0, requests[1].1.path()),
        (&Method::GET, "/article")
    );
    assert_eq!(requests[1].1.host(), Some("pacing.example.com"));
    assert_eq!(
        requests[1].3.get(axum::http::header::AUTHORIZATION).and_then(|value| value.to_str().ok()),
        Some("Basic dXNlcjpwYXNz"),
        "the original request must actually carry the synthetic credentials before the cross-origin guard can be tested"
    );
    assert_eq!(
        (&requests[2].0, requests[2].1.path()),
        (&Method::GET, "/landing")
    );
    assert_eq!(requests[2].1.host(), Some("destination.example.net"));
    assert!(
        requests[2].2.duration_since(requests[1].2) >= Duration::from_millis(1950),
        "cross-host redirect ignored Retry-After: 2: {requests:?}"
    );
    assert!(
        !requests[2]
            .3
            .contains_key(axum::http::header::AUTHORIZATION),
        "untrusted Location userinfo must not add Authorization: {requests:?}"
    );
    assert!(
        !requests[2].1.to_string().contains("fixture-user")
            && !requests[2].1.to_string().contains("fixture-password"),
        "untrusted Location userinfo must not remain in the outgoing URL: {requests:?}"
    );
    for scope in ["site", "content"] {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            temp.path().join(scope).join("health-summary.json"),
        )?)?;
        assert_eq!(report["total_checked"], 1);
        assert_eq!(report["live_count"], 1, "{scope}: {report}");
    }
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("check-status.json"))?)?;
    assert_eq!(status["status"], "completed");
    assert_eq!(status["phase"], "content");
    Ok(())
}

#[tokio::test]
async fn slow_content_check_paces_site_head_to_get_fallback() -> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri| {
        let observed = Arc::clone(&observed);
        async move {
            let head = method == Method::HEAD;
            observed
                .lock()
                .await
                .push((method, uri.path().to_owned(), Instant::now()));
            if head {
                StatusCode::METHOD_NOT_ALLOWED.into_response()
            } else {
                axum::response::Html(HEALTHY_HTML).into_response()
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = run_content_check(&proxy, true).await;
    server.abort();
    let (temp, output) = result?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 3, "unexpected requests: {requests:?}");
    assert_eq!(
        (&requests[0].0, requests[0].1.as_str()),
        (&Method::HEAD, "/")
    );
    assert_eq!(
        (&requests[1].0, requests[1].1.as_str()),
        (&Method::GET, "/")
    );
    assert_eq!(
        (&requests[2].0, requests[2].1.as_str()),
        (&Method::GET, "/article")
    );
    assert!(
        requests[1].2.duration_since(requests[0].2) >= Duration::from_millis(1950),
        "HEAD-to-GET fallback bypassed slow-mode request pacing: {requests:?}"
    );
    for scope in ["site", "content"] {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            temp.path().join(scope).join("health-summary.json"),
        )?)?;
        assert_eq!(report["total_checked"], 1);
        assert_eq!(report["live_count"], 1, "{scope}: {report}");
    }
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("check-status.json"))?)?;
    assert_eq!(status["status"], "completed");
    assert_eq!(status["phase"], "content");
    Ok(())
}
