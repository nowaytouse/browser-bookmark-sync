use std::{
    error::Error,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use browser_bookmark_sync::health::{
    db::HealthDatabase,
    engine::{HealthEngine, HealthEngineOptions},
    models::ProbeTarget,
    state_machine::CURRENT_HEALTH_RULE_VERSION,
    targets::ProbeScope,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};

const URL: &str = "http://93.184.216.34/article";
const BOOKMARK_ID: &str = "scan-lifecycle-article";
const ARTICLE: &str = "<!doctype html><html><head><title>Article</title></head><body><main><h1>Article</h1><p>Available article text.</p></main></body></html>";

struct AbortOnDrop(JoinHandle<io::Result<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn start_proxy(
    response: Option<Vec<u8>>,
) -> Result<(String, oneshot::Receiver<()>, AbortOnDrop), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (received_tx, received_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        let mut reader = BufReader::new(socket);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "proxy received no request line",
            ));
        }
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "proxy request ended before headers completed",
                ));
            }
            if line == "\r\n" {
                break;
            }
        }
        let _ = received_tx.send(());
        if let Some(response) = response {
            reader.get_mut().write_all(&response).await?;
        } else {
            std::future::pending::<()>().await;
        }
        Ok(())
    });

    Ok((
        format!("http://{address}"),
        received_rx,
        AbortOnDrop(server),
    ))
}

fn engine_options(
    proxy_url: String,
    db: std::path::PathBuf,
    output_dir: std::path::PathBuf,
) -> HealthEngineOptions {
    HealthEngineOptions {
        proxy_url: Some(proxy_url),
        timeout_secs: 5,
        concurrency: 1,
        probe_scope: ProbeScope::Content,
        history_db_path: Some(db),
        output_dir: Some(output_dir),
        max_attempts: 1,
        retry_interval_secs: 0,
        ..HealthEngineOptions::default()
    }
}

fn target() -> Result<ProbeTarget, io::Error> {
    ProbeTarget::from_url(BOOKMARK_ID, URL)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))
}

#[tokio::test]
async fn cancelled_probe_keeps_planned_scan_member_unfinished_without_health_evidence(
) -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db_path = temp.path().join("health.sqlite");
    let (proxy_url, request_received, server) = start_proxy(None).await?;
    let engine = HealthEngine::new(engine_options(
        proxy_url,
        db_path.clone(),
        temp.path().join("reports"),
    ))
    .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let request_seen = Arc::new(AtomicBool::new(false));
    let request_seen_by_timeout = request_seen.clone();
    let scan = engine.run_probes_with_results(vec![target()?], |_, _, _| {});

    let timed = tokio::time::timeout(Duration::from_secs(3), async move {
        tokio::pin!(scan);
        tokio::select! {
            _ = &mut scan => Err::<(), String>("scan finished before cancellation".into()),
            ready = request_received => {
                ready.map_err(|error| error.to_string())?;
                request_seen_by_timeout.store(true, Ordering::SeqCst);
                // Keep the scan alive until the deadline, not just until select picks this branch.
                scan.await.map(|_| ())
            }
        }
    })
    .await;
    assert!(
        timed.is_err(),
        "held request should be cancelled by timeout"
    );
    assert!(
        request_seen.load(Ordering::SeqCst),
        "the proxy must receive the request before cancellation"
    );

    drop(server);
    drop(engine);
    let db = HealthDatabase::open_read_only(&db_path)?;
    let history = db
        .get_url_scan_history(URL, 10, 0)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "scan ledger is unavailable"))?;
    assert_eq!(history.len(), 1);
    let scan = &history[0];
    assert_eq!(scan.scope, "content");
    assert!(!scan.completed);
    assert!(scan.finished_at.is_none());
    assert!(scan.result.is_none());
    assert_eq!(scan.planned_targets, 1);
    assert!(db.get_url_history_details(URL, 10, 0)?.is_empty());
    assert!(db
        .get_qualifying_run_evidences(BOOKMARK_ID, None, CURRENT_HEALTH_RULE_VERSION,)?
        .is_empty());
    Ok(())
}

#[tokio::test]
async fn report_export_failure_keeps_completed_target_evidence_but_not_scan_completion(
) -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db_path = temp.path().join("health.sqlite");
    let output_file = temp.path().join("not-a-report-directory");
    std::fs::write(&output_file, b"preserve this file")?;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        ARTICLE.len(),
        ARTICLE
    )
    .into_bytes();
    let (proxy_url, request_received, server) = start_proxy(Some(response)).await?;
    let engine = HealthEngine::new(engine_options(
        proxy_url,
        db_path.clone(),
        output_file.clone(),
    ))
    .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.run_probes_with_results(vec![target()?], |_, _, _| {}),
    )
    .await?;
    let error = result.err().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "report export to an existing file unexpectedly succeeded",
        )
    })?;
    assert!(
        error.starts_with("Report generation error:"),
        "expected report export failure, got: {error}"
    );
    request_received.await?;
    assert_eq!(std::fs::read(&output_file)?, b"preserve this file");

    drop(server);
    drop(engine);
    let db = HealthDatabase::open_read_only(&db_path)?;
    let scans = db
        .get_url_scan_history(URL, 10, 0)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "scan ledger is unavailable"))?;
    assert_eq!(scans.len(), 1);
    let scan = &scans[0];
    assert_eq!(scan.scope, "content");
    assert_eq!(scan.planned_targets, 1);
    assert!(!scan.completed);
    assert!(scan.finished_at.is_none());
    let result = scan
        .result
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target result was not retained"))?;
    let observations = result["observations"]
        .as_array()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "observations missing"))?;
    let source_observation = observations
        .iter()
        .find(|observation| observation["source"].is_object())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "original source was not retained")
        })?;
    assert_eq!(source_observation["status_code"], 200);
    assert_eq!(source_observation["actual_request"], "yes");
    assert_eq!(source_observation["cache_hit"], false);
    assert!(source_observation["source"]["run_id"].is_string());
    assert!(source_observation["source"]["check_id"].is_string());
    assert!(source_observation["source"]["config_hash"].is_string());
    assert!(source_observation["source"]["rule_version"].is_string());
    Ok(())
}
