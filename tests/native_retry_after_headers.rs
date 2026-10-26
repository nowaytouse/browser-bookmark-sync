use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    body::{Body, Bytes},
    http::{header::CONTENT_TYPE, StatusCode, Uri},
    response::IntoResponse,
};
use browser_bookmark_sync::health::{
    HealthEngine, HealthEngineOptions, HealthVerdict, ProbeScope, ProbeTarget,
};
use chrono::Utc;
use futures::StreamExt;
use tokio::{
    sync::{Mutex, Notify},
    time::Instant,
};

struct AbortServer(tokio::task::AbortHandle);

impl Drop for AbortServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn retry_after_headers_delay_new_requests_before_the_limited_body_finishes(
) -> Result<(), Box<dyn Error>> {
    // This proxy serves its own synthetic replies and never forwards requests
    // or resolves the target hostname.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let limited_body_time = Arc::new(Mutex::new(None));
    let limited_body_started = Arc::new(Notify::new());
    let observed = Arc::clone(&requests);
    let body_time = Arc::clone(&limited_body_time);
    let body_started = Arc::clone(&limited_body_started);
    let app = axum::Router::new().fallback(move |uri: Uri| {
        let observed = Arc::clone(&observed);
        let body_time = Arc::clone(&body_time);
        let body_started = Arc::clone(&body_started);
        async move {
            observed
                .lock()
                .await
                .push((uri.path().to_owned(), Instant::now(), Utc::now()));
            match uri.path() {
                "/limited" => {
                    let prefix = futures::stream::once(async move {
                        *body_time.lock().await = Some((Instant::now(), Utc::now()));
                        // Signal from the body stream, not merely handler entry.
                        body_started.notify_one();
                        Ok::<_, std::io::Error>(Bytes::from_static(b"<html><body>slow rate limit"))
                    });
                    let body = prefix.chain(futures::stream::pending());
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        [(CONTENT_TYPE, "text/html"), (axum::http::header::RETRY_AFTER, "2")],
                        Body::from_stream(body),
                    )
                        .into_response()
                }
                "/next" => axum::response::Html("<!doctype html><html><head><title>Healthy local article</title></head><body>A complete article for testing immediate publication of server cooldowns.</body></html>").into_response(),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let _server_guard = AbortServer(server.abort_handle());
    let engine = HealthEngine::new(HealthEngineOptions {
        proxy_url: Some(proxy),
        concurrency: 2,
        timeout_secs: 5,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        max_attempts: 1,
        history_db_path: None,
        output_dir: None,
        secondary_backends: Vec::new(),
        ..HealthEngineOptions::default()
    })?;
    let limited = ProbeTarget::from_url("limited", "http://cooldown.example.com/limited")?;
    let mut in_flight = Box::pin(engine.run_probes_with_results(vec![limited], |_, _, _| {}));
    tokio::select! {
        began = tokio::time::timeout(Duration::from_secs(3), limited_body_started.notified()) => {
            began.map_err(|_| "the rate-limited body never began before the fixture deadline")?;
        }
        result = &mut in_flight => {
            return Err(format!("the deliberately unfinished rate-limited response completed unexpectedly: {result:?}").into());
        }
    }
    let (limited_at, limited_utc) = limited_body_time
        .lock()
        .await
        .as_ref()
        .copied()
        .ok_or("the rate-limited body did not record its send time")?;
    let next = ProbeTarget::from_url("next", "http://cooldown.example.com/next")?;
    let mut next_run = Box::pin(engine.run_probes_with_results(vec![next], |_, _, _| {}));
    // Keep polling the first run so it can receive its response headers and
    // publish the cooldown, while its unfinished body continues holding one slot.
    let (report, results) = tokio::select! {
        next_result = tokio::time::timeout(Duration::from_secs(3), &mut next_run) => {
            next_result.map_err(|_| "the follow-up exceeded the bounded cooldown fixture deadline")??
        }
        result = &mut in_flight => {
            return Err(format!("the deliberately unfinished body finished before the follow-up: {result:?}").into());
        }
    };
    drop(in_flight);
    assert_eq!(report.total_checked, 1);
    assert_eq!(report.live_count, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].2, HealthVerdict::Live);
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 2, "unexpected requests: {requests:?}");
    assert_eq!(requests[0].0, "/limited");
    assert_eq!(requests[1].0, "/next");
    assert!(
        requests[1].1.duration_since(limited_at) >= Duration::from_millis(1950),
        "Retry-After headers were not shared until the unfinished body completed: {requests:?}"
    );
    let native = results[0]
        .3
        .iter()
        .find(|observation| observation.backend == "native_reqwest")
        .ok_or("the follow-up had no native observation")?;
    assert!(
        native.started_at >= limited_utc + chrono::Duration::milliseconds(1900),
        "the follow-up evidence was backdated to queue entry instead of its actual send: {native:?}"
    );
    Ok(())
}
