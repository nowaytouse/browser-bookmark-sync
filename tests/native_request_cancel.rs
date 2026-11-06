use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    body::{Body, Bytes},
    http::{header::CONTENT_TYPE, StatusCode, Uri},
    response::IntoResponse,
};
use browser_bookmark_sync::health::{
    HealthEngine, HealthEngineOptions, HealthVerdict, ProbeScope, ProbeTarget,
};
use futures::StreamExt;
use tokio::sync::{Mutex, Notify};

struct AbortServer(tokio::task::AbortHandle);

impl Drop for AbortServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn cancelled_content_body_releases_the_same_engine_host_for_a_later_request(
) -> Result<(), Box<dyn Error>> {
    assert_single_slot_body_cancellation(
        "http://cancel.example.com/recovered",
        Duration::from_millis(2200),
    )
    .await
}

#[tokio::test]
async fn slow_mode_respects_a_requested_single_global_slot() -> Result<(), Box<dyn Error>> {
    assert_single_slot_body_cancellation(
        "http://recovered.example.net/recovered",
        Duration::from_millis(400),
    )
    .await
}

async fn assert_single_slot_body_cancellation(
    recovered_url: &str,
    blocked_duration: Duration,
) -> Result<(), Box<dyn Error>> {
    // All requests terminate at this loopback-only proxy; it never forwards or
    // resolves the synthetic bookmark hostname.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let request_started = Arc::new(Notify::new());
    let paths = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&paths);
    let started = Arc::clone(&request_started);
    let app = axum::Router::new().fallback(move |uri: Uri| {
        let observed = Arc::clone(&observed);
        let started = Arc::clone(&started);
        async move {
            observed.lock().await.push(uri.path().to_owned());
            match uri.path() {
                "/hang" => {
                    let prefix = futures::stream::once(async move {
                        // Signal only when the server begins yielding the body,
                        // not when the handler merely accepts the request.
                        started.notify_one();
                        Ok::<_, std::io::Error>(Bytes::from_static(b"<html><body>unfinished"))
                    });
                    let body = prefix.chain(futures::stream::pending());
                    ([(CONTENT_TYPE, "text/html")], Body::from_stream(body)).into_response()
                }
                "/recovered" => axum::response::Html("<!doctype html><html><head><title>Recovered local article</title></head><body>A complete healthy response from the cancellation regression fixture.</body></html>").into_response(),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let _server_guard = AbortServer(server.abort_handle());
    let engine = HealthEngine::new(HealthEngineOptions {
        proxy_url: Some(proxy),
        concurrency: 1,
        timeout_secs: 5,
        probe_scope: ProbeScope::Content,
        slow_mode: true,
        max_attempts: 1,
        history_db_path: None,
        output_dir: None,
        secondary_backends: Vec::new(),
        ..HealthEngineOptions::default()
    })?;
    let target = ProbeTarget::from_url("hang", "http://cancel.example.com/hang")?;
    let mut in_flight = Box::pin(engine.run_probes_with_results(vec![target], |_, _, _| {}));
    tokio::select! {
        began = tokio::time::timeout(Duration::from_secs(3), request_started.notified()) => {
            began.map_err(|_| "the hanging body never began before the fixture deadline")?;
        }
        result = &mut in_flight => {
            return Err(format!("the deliberately unfinished body completed unexpectedly: {result:?}").into());
        }
    }
    let target = ProbeTarget::from_url("recovered", recovered_url)?;
    let mut recovery = Box::pin(engine.run_probes_with_results(vec![target], |_, _, _| {}));
    // The same-host case waits beyond slow pacing; the different-host case
    // isolates the shared global slot from per-host interval/concurrency limits.
    assert!(
        tokio::time::timeout(blocked_duration, &mut recovery)
            .await
            .is_err(),
        "the unfinished body released this engine's request slot before cancellation"
    );
    assert_eq!(paths.lock().await.as_slice(), ["/hang"]);
    drop(in_flight);

    let (report, results) = tokio::time::timeout(Duration::from_secs(3), &mut recovery)
        .await
        .map_err(|_| {
            "cancelling the body left this engine's host or global request slot occupied"
        })??;
    assert_eq!(report.total_checked, 1);
    assert_eq!(report.live_count, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].2, HealthVerdict::Live);
    assert_eq!(paths.lock().await.as_slice(), ["/hang", "/recovered"]);
    Ok(())
}
