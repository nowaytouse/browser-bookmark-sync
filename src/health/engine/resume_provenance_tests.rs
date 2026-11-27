//! Public engine acceptance for preserved and rejected history provenance.

use super::*;
use chrono::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct LocalProxy {
    url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Drop for LocalProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn local_proxy() -> std::io::Result<LocalProxy> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await?;
            let mut header = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let length = socket.read(&mut buffer).await?;
                if length == 0 || header.len() + length > 8192 {
                    return Err(std::io::Error::other("incomplete synthetic proxy request"));
                }
                header.extend_from_slice(&buffer[..length]);
            }
            observed.fetch_add(1, Ordering::SeqCst);
            if String::from_utf8_lossy(&header)
                .lines()
                .next()
                .is_some_and(|s| s.contains("/memory-protected"))
            {
                socket
                    .write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await?;
                continue;
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlive")
                .await?;
        }
    });
    Ok(LocalProxy {
        url,
        requests,
        task,
    })
}

fn source_context(proxy: &LocalProxy, database: &std::path::Path) -> ProbeContext {
    ProbeContext {
        run_id: "original-interrupted-run".into(),
        check_id: "original-precise-check".into(),
        proxy_url: Some(proxy.url.clone()),
        timeout_secs: 2,
        slow_mode: false,
        probe_scope: ProbeScope::Content,
        dns_policy_hash: DnsPolicy::default().hash().unwrap(),
        allow_private_ips: false,
        history_db_path: Some(database.into()),
        secondary_backends: Vec::new(),
        artifact_dir: None,
    }
}

fn engine_for(
    proxy: &LocalProxy,
    database: PathBuf,
    output: PathBuf,
) -> Result<HealthEngine, String> {
    HealthEngine::new(HealthEngineOptions {
        proxy_url: Some(proxy.url.clone()),
        timeout_secs: 2,
        concurrency: 1,
        history_db_path: Some(database),
        output_dir: Some(output),
        max_attempts: 1,
        retry_interval_secs: 0,
        ..Default::default()
    })
}

#[tokio::test]
async fn green_memory_survives_reopening_without_another_request_or_refreshing_evidence(
) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = local_proxy().await?;
    let dir = tempfile::tempdir()?;
    let memory = dir.path().join("green.sqlite");
    let url = "http://93.184.216.34/article?edition=1";
    let html = dir.path().join("bookmarks.html");
    std::fs::write(
        &html,
        format!(r#"<DL><DT><A HREF="{url}">Article</A></DL>"#),
    )?;
    let mut original = None;
    for n in 0..3 {
        let db = dir.path().join(format!("batch-{n}.db"));
        let engine = engine_for(&proxy, db.clone(), dir.path().join(format!("reports-{n}")))?
            .with_green_memory(Some(&memory))?;
        // A fresh export may assign a different bookmark ID to exactly the same page.
        let before = proxy.requests.load(Ordering::SeqCst);
        let (_, rows) = engine
            .run_probes_with_results(
                vec![ProbeTarget::from_url(format!("item-{n}"), url)?],
                |_, _, _| {},
            )
            .await?;
        let obs = &rows[0].3[0];
        assert_eq!(rows[0].2, HealthVerdict::Live);
        if n == 0 {
            assert!(proxy.requests.load(Ordering::SeqCst) > before);
            original = Some((obs.check_id.clone(), obs.started_at, obs.finished_at));
        } else {
            assert_eq!(before, proxy.requests.load(Ordering::SeqCst));
            assert_eq!(obs.backend, "green_memory");
            assert!(obs.cache_hit && obs.actual_request == "no");
            assert_eq!(
                Some((obs.check_id.clone(), obs.started_at, obs.finished_at)),
                original
            );
            assert!(HealthDatabase::open(&db)?.get_url_history(url)?.is_empty());
            let doc = crate::health::review::ReviewDocument::load(&html, Some(&db), None)?;
            assert_eq!(doc.items[0].verdict, "live");
            assert_eq!(doc.items[0].total_checks, 0);
            assert_eq!(doc.items[0].evidence[0].backend, "green_memory");
        }
    }
    // A different path/query is not covered; neither is a protected result.
    for n in 3..5 {
        let engine = engine_for(
            &proxy,
            dir.path().join(format!("{n}.db")),
            dir.path().join(format!("r{n}")),
        )?
        .with_green_memory(Some(&memory))?;
        let (_, rows) = engine
            .run_probes_with_results(
                vec![
                    ProbeTarget::from_url("different", "http://93.184.216.34/article?edition=2")?,
                    ProbeTarget::from_url("protected", "http://93.184.216.34/memory-protected")?,
                ],
                |_, _, _| {},
            )
            .await?;
        let find = |id: &str| rows.iter().find(|r| r.0 == id).unwrap();
        assert_eq!(find("different").3[0].cache_hit, n == 4);
        assert_eq!(find("protected").2, HealthVerdict::Protected);
        assert!(find("protected")
            .3
            .iter()
            .any(|o| o.actual_request == "yes" && !o.cache_hit));
    }
    Ok(())
}

#[tokio::test]
async fn interrupted_positive_reuse_preserves_exact_real_attempt_source(
) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = local_proxy().await?;
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("health.sqlite");
    let context = source_context(&proxy, &database);
    let target = ProbeTarget::from_url("retained", "http://93.184.216.34/retained")?;
    let began = Utc::now() - Duration::minutes(5);
    let finished = began + Duration::milliseconds(37);
    let real = BackendObservation {
        backend: "native_reqwest".into(),
        run_id: context.run_id.clone(),
        check_id: context.check_id.clone(),
        started_at: began,
        finished_at: finished,
        status_code: Some(200),
        initial_url: target.original_url.clone(),
        ..Default::default()
    };
    let cached = BackendObservation {
        backend: "hickory_dns".into(),
        actual_request: "no".into(),
        cache_hit: true,
        started_at: Utc::now() - Duration::minutes(1),
        finished_at: Utc::now() - Duration::minutes(1),
        dns_rcode: Some("NOERROR".into()),
        ..real.clone()
    };
    let db = HealthDatabase::open(&database)?;
    db.record_run_start(&context)?;
    db.record_check_observations(
        &context.run_id,
        &context.check_id,
        &target.bookmark_id,
        &target.original_url,
        HealthVerdict::Live,
        &[real, cached],
    )?;
    db.conn.execute(
        "UPDATE health_runs SET started_at=?1 WHERE run_id=?2",
        rusqlite::params![(began - Duration::seconds(1)).to_rfc3339(), context.run_id],
    )?;
    drop(db);

    let engine = engine_for(&proxy, database.clone(), temp.path().join("report"))?;
    let (report, results) = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        engine.run_probes_with_results(vec![target.clone()], |_, _, _| {}),
    )
    .await??;
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 0);
    assert_ne!(report.run_id, context.run_id);
    let reused = &results[0].3[0];
    assert_eq!(reused.request_method, "REUSE");
    assert_eq!(reused.actual_request, "no");
    assert!(reused.cache_hit);
    assert_eq!(reused.started_at, began);
    assert_eq!(reused.finished_at, finished);
    assert_eq!(reused.run_id, context.run_id);
    assert_eq!(reused.check_id, context.check_id);
    let source = reused
        .source
        .as_ref()
        .expect("reuse must identify its original source");
    assert_eq!(source.run_id, context.run_id);
    assert_eq!(source.check_id, context.check_id);
    assert_eq!(
        source.config_hash,
        crate::health::models::ConfigHashInput::from_probe_context(&context).stable_hash()?
    );
    assert_eq!(
        source.rule_version,
        crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION
    );
    let history = HealthDatabase::open(&database)?.get_url_history(&target.original_url)?;
    assert_eq!(
        history.len(),
        1,
        "reuse must not fabricate another raw check"
    );
    assert_eq!(history[0].0, context.run_id);
    Ok(())
}

#[tokio::test]
async fn invalid_or_nonreal_source_attempts_are_reprobed_not_refreshed(
) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = local_proxy().await?;
    let now = Utc::now();
    let valid_start = (now - Duration::minutes(2)).to_rfc3339();
    let valid_finish = (now - Duration::minutes(1)).to_rfc3339();
    let cases = [
        (
            "future-start",
            Some((now + Duration::hours(1)).to_rfc3339()),
            Some((now + Duration::hours(2)).to_rfc3339()),
            "yes",
            false,
        ),
        (
            "future-finish",
            Some(valid_start.clone()),
            Some((now + Duration::hours(1)).to_rfc3339()),
            "yes",
            false,
        ),
        (
            "expired-start",
            Some((now - Duration::hours(25)).to_rfc3339()),
            Some(valid_finish.clone()),
            "yes",
            false,
        ),
        (
            "expired-finish",
            Some((now - Duration::hours(26)).to_rfc3339()),
            Some((now - Duration::hours(25)).to_rfc3339()),
            "yes",
            false,
        ),
        (
            "malformed-start",
            Some("not-a-time".into()),
            Some(valid_finish.clone()),
            "yes",
            false,
        ),
        (
            "malformed-finish",
            Some(valid_start.clone()),
            Some("not-a-time".into()),
            "yes",
            false,
        ),
        (
            "missing-start",
            None,
            Some(valid_finish.clone()),
            "yes",
            false,
        ),
        (
            "missing-finish",
            Some(valid_start.clone()),
            None,
            "yes",
            false,
        ),
        (
            "inverted",
            Some(valid_finish.clone()),
            Some(valid_start.clone()),
            "yes",
            false,
        ),
        (
            "cached-only",
            Some(valid_start.clone()),
            Some(valid_finish.clone()),
            "no",
            true,
        ),
    ];
    for (index, (case, started, finished, actual, cache_hit)) in cases.into_iter().enumerate() {
        let temp = tempfile::tempdir()?;
        let database = temp.path().join("health.sqlite");
        let context = source_context(&proxy, &database);
        let target = ProbeTarget::from_url(case, &format!("http://93.184.216.34/{case}"))?;
        let db = HealthDatabase::open(&database)?;
        let mut older_context = context.clone();
        older_context.run_id.push_str("-older");
        older_context.check_id.push_str("-older");
        db.record_run_start(&older_context)?;
        db.record_check_observations(
            &older_context.run_id,
            &older_context.check_id,
            &target.bookmark_id,
            &target.original_url,
            HealthVerdict::Live,
            &[BackendObservation {
                backend: "native_reqwest".into(),
                started_at: now - Duration::minutes(12),
                finished_at: now - Duration::minutes(11),
                status_code: Some(200),
                initial_url: target.original_url.clone(),
                ..Default::default()
            }],
        )?;
        db.conn.execute(
            "UPDATE health_runs SET started_at=?1 WHERE run_id=?2",
            rusqlite::params![
                (now - Duration::minutes(13)).to_rfc3339(),
                older_context.run_id
            ],
        )?;
        db.record_run_start(&context)?;
        db.record_check_observations(
            &context.run_id,
            &context.check_id,
            &target.bookmark_id,
            &target.original_url,
            HealthVerdict::Live,
            &[BackendObservation {
                backend: "native_reqwest".into(),
                status_code: Some(200),
                initial_url: target.original_url.clone(),
                ..Default::default()
            }],
        )?;
        // Only synthetic fixtures corrupt source metadata; the public engine must not repair it as now.
        db.conn.execute(
            "UPDATE health_attempts SET started_at=?1,finished_at=?2,actual_request=?3,cache_hit=?4 WHERE check_id=?5",
            rusqlite::params![started, finished, actual, i32::from(cache_hit), context.check_id],
        )?;
        drop(db);
        let engine = engine_for(&proxy, database, temp.path().join("report"))?;
        let (report, results) = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            engine.run_probes_with_results(vec![target], |_, _, _| {}),
        )
        .await??;
        assert_eq!(report.live_count, 1, "{case}");
        assert_eq!(
            proxy.requests.load(Ordering::SeqCst),
            index + 1,
            "{case} was reused"
        );
        let observation = &results[0].3[0];
        assert_ne!(observation.request_method, "REUSE", "{case}");
        assert_eq!(observation.actual_request, "yes", "{case}");
        assert!(!observation.cache_hit, "{case}");
        let source = observation
            .source
            .as_ref()
            .expect("new real probe source missing");
        assert_eq!(source.run_id, observation.run_id, "{case}");
        assert_eq!(source.check_id, observation.check_id, "{case}");
        assert_ne!(source.run_id, context.run_id, "{case}");
    }
    Ok(())
}

#[tokio::test]
async fn green_memory_rechecks_after_settings_change_expiry_or_invalid_evidence(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let memory = temp.path().join("greens.sqlite");
    let first = local_proxy().await?;
    let second = local_proxy().await?;
    let target = ProbeTarget::from_url("same-page", "http://93.184.216.34/memory-policy")?;
    for i in 0..7 {
        let proxy = if i == 0 { &first } else { &second };
        let db = temp.path().join(format!("batch-{i}.sqlite"));
        if i >= 4 {
            let conn = rusqlite::Connection::open(&memory)?;
            let timestamp = match i {
                4 => (Utc::now() - Duration::hours(25)).to_rfc3339(),
                5 => (Utc::now() + Duration::hours(25)).to_rfc3339(),
                _ => "damaged".into(),
            };
            assert!(
                conn.execute(
                    "UPDATE greens SET evidence=json_set(evidence,'$.started',?1,'$.finished',?1)",
                    [timestamp]
                )? > 0
            );
        }
        let engine = HealthEngine::new(HealthEngineOptions {
            history_db_path: Some(db),
            proxy_url: Some(proxy.url.clone()),
            probe_scope: if i == 2 {
                ProbeScope::Site
            } else {
                ProbeScope::Content
            },
            dns_policy: if i >= 3 {
                DnsPolicy {
                    dns_fallback_on_error: true,
                    ..Default::default()
                }
            } else {
                DnsPolicy::default()
            },
            output_dir: Some(temp.path().join(format!("report-{i}"))),
            max_attempts: 1,
            ..Default::default()
        })?
        .with_green_memory(Some(&memory))?;
        let before = proxy.requests.load(Ordering::SeqCst);
        let (_, results) = engine
            .run_probes_with_results(vec![target.clone()], |_, _, _| {})
            .await?;
        assert!(
            proxy.requests.load(Ordering::SeqCst) > before,
            "case {i} unexpectedly skipped"
        );
        assert_eq!(results[0].2, HealthVerdict::Live);
    }
    Ok(())
}
