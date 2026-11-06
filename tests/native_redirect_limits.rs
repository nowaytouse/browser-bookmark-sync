use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    http::{StatusCode, Uri},
    response::IntoResponse,
};
use browser_bookmark_sync::health::{
    models::BackendObservation, HealthEngine, HealthEngineOptions, HealthVerdict, ProbeScope,
    ProbeTarget,
};
use tokio::sync::Mutex;

struct AbortServer(tokio::task::AbortHandle);

impl Drop for AbortServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct RedirectOutcome {
    verdict: HealthVerdict,
    observation: BackendObservation,
    paths: Vec<String>,
}

#[tokio::test]
async fn redirect_chain_keeps_one_timeout_budget_instead_of_resetting_per_hop(
) -> Result<(), Box<dyn Error>> {
    let outcome = probe_redirects(1, Duration::from_secs(5), "/loop").await?;
    assert!(!outcome.paths.is_empty(), "the fixture was never requested");
    assert!(
        outcome.paths.len() <= 5,
        "a one-second chain with 200 ms host pacing amplified requests: {:?}",
        outcome.paths
    );
    assert_eq!(outcome.observation.error_kind.as_deref(), Some("Timeout"));
    assert_ne!(outcome.verdict, HealthVerdict::ConfirmedDead);
    Ok(())
}

#[tokio::test]
async fn redirect_loop_preserves_the_ten_request_boundary() -> Result<(), Box<dyn Error>> {
    let outcome = probe_redirects(10, Duration::from_secs(6), "/loop").await?;
    assert_eq!(
        outcome.paths.len(),
        10,
        "unexpected requests: {:?}",
        outcome.paths
    );
    assert_eq!(outcome.paths[0], "/article");
    assert!(outcome.paths[1..].iter().all(|path| path == "/loop"));
    assert_eq!(outcome.observation.redirect_count, 9);
    assert_eq!(
        outcome.observation.error_kind.as_deref(),
        Some("HttpError: Too many redirects")
    );
    assert_ne!(outcome.verdict, HealthVerdict::ConfirmedDead);
    Ok(())
}

#[tokio::test]
async fn redirect_to_a_private_literal_stops_before_a_second_proxy_request(
) -> Result<(), Box<dyn Error>> {
    let outcome = probe_redirects(5, Duration::from_secs(5), "http://127.0.0.1:9/private").await?;
    assert_eq!(outcome.paths.as_slice(), ["/article"]);
    assert!(
        outcome
            .observation
            .error_kind
            .as_deref()
            .is_some_and(|error| error.starts_with("SecurityBlocked:")),
        "private redirect did not preserve the security block: {:?}",
        outcome.observation
    );
    assert_eq!(outcome.observation.redirect_count, 0);
    assert_ne!(outcome.verdict, HealthVerdict::ConfirmedDead);
    Ok(())
}

async fn probe_redirects(
    timeout_secs: u64,
    outer_timeout: Duration,
    location: &'static str,
) -> Result<RedirectOutcome, Box<dyn Error>> {
    // This local proxy returns synthetic responses itself; it never forwards
    // requests or resolves either the source or redirect hostname.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let paths = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&paths);
    let app = axum::Router::new().fallback(move |uri: Uri| {
        let observed = Arc::clone(&observed);
        async move {
            observed.lock().await.push(uri.path().to_owned());
            (StatusCode::FOUND, [("location", location)], "").into_response()
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let _server_guard = AbortServer(server.abort_handle());
    let engine = HealthEngine::new(HealthEngineOptions {
        proxy_url: Some(proxy),
        concurrency: 1,
        timeout_secs,
        probe_scope: ProbeScope::Content,
        slow_mode: false,
        max_attempts: 1,
        allow_private_ips: false,
        history_db_path: None,
        output_dir: None,
        secondary_backends: Vec::new(),
        ..HealthEngineOptions::default()
    })?;
    let target = ProbeTarget::from_url("redirect", "http://redirect.example.com/article")?;
    let (report, results) = tokio::time::timeout(
        outer_timeout,
        engine.run_probes_with_results(vec![target], |_, _, _| {}),
    )
    .await
    .map_err(|_| "the redirect probe exceeded its bounded fixture deadline")??;
    assert_eq!(report.total_checked, 1);
    assert_eq!(results.len(), 1);
    let Some((_, _, verdict, observations)) = results.into_iter().next() else {
        return Err("the redirect probe returned no current result".into());
    };
    let observation = observations
        .into_iter()
        .find(|observation| observation.backend == "native_reqwest")
        .ok_or("the redirect probe returned no native observation")?;
    let paths = paths.lock().await.clone();
    Ok(RedirectOutcome {
        verdict,
        observation,
        paths,
    })
}
