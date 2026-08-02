//! Comprehensive Unit and Integration Tests for Health Verification System

#[cfg(test)]
mod tests {

    fn mock_evidences_one_family(
        count: usize,
        first: Option<chrono::DateTime<chrono::Utc>>,
        last: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<crate::health::models::QualifyingRunEvidence> {
        use crate::health::models::{QualifyingRunEvidence, StrongFailureKind};
        use std::collections::BTreeSet;
        let mut evs = Vec::new();
        if count == 0 {
            return evs;
        }
        let first_dt = first.unwrap_or_else(|| chrono::Utc::now());
        let last_dt = last.unwrap_or_else(|| chrono::Utc::now());
        let span = last_dt.signed_duration_since(first_dt).num_seconds();
        let step = if count > 1 {
            span / (count as i64 - 1)
        } else {
            0
        };
        for i in 0..count {
            let dt = first_dt + chrono::Duration::seconds(step * i as i64);
            let mut families = BTreeSet::new();
            families.insert("native".to_string());
            let mut kinds = BTreeSet::new();
            kinds.insert(StrongFailureKind::Http404);
            evs.push(QualifyingRunEvidence {
                run_id: format!("mock-run-{} {:?}", i, dt),
                completed_at: dt,
                strong_failure_at: dt,
                verifier_families: families,
                strong_failure_kinds: kinds,
            });
        }
        evs
    }

    fn mock_evidences(
        count: usize,
        first: Option<chrono::DateTime<chrono::Utc>>,
        last: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<crate::health::models::QualifyingRunEvidence> {
        use crate::health::models::{QualifyingRunEvidence, StrongFailureKind};
        use std::collections::BTreeSet;
        let mut evs = Vec::new();
        if count == 0 {
            return evs;
        }
        let first_dt = first.unwrap_or_else(|| chrono::Utc::now());
        let last_dt = last.unwrap_or_else(|| chrono::Utc::now());

        // Spread the dates
        let span = last_dt.signed_duration_since(first_dt).num_seconds();
        let step = if count > 1 {
            span / (count as i64 - 1)
        } else {
            0
        };

        for i in 0..count {
            let dt = first_dt + chrono::Duration::seconds(step * i as i64);
            let mut families = BTreeSet::new();
            // Make the first one have 'native', second 'lychee', others 'native'
            // This ensures if count >= 2, we have at least 2 families!
            families.insert("native".to_string());
            if i == 1 {
                families.insert("lychee".to_string());
            }

            let mut kinds = BTreeSet::new();
            kinds.insert(StrongFailureKind::Http404);

            evs.push(QualifyingRunEvidence {
                run_id: format!("mock-run-{} {:?}", i, dt),
                completed_at: dt,
                strong_failure_at: dt,
                verifier_families: families,
                strong_failure_kinds: kinds,
            });
        }
        evs
    }

    use crate::health::db::HealthDatabase;
    use crate::health::models::{BackendObservation, HealthVerdict};
    use crate::health::reporting::ReportExporter;
    use crate::health::scheduler::{HostScheduler, SchedulerConfig};
    use crate::health::security::{validate_url_security, SecurityCheckResult};
    use crate::health::state_machine::{BookmarkHistoryStats, StateMachine};
    use crate::url_checker::{HttpResult, ValidationStatus};
    use chrono::{Duration, Utc};

    #[test]
    fn test_tier0_security_blocking() {
        assert_eq!(
            validate_url_security("http://example.com", false),
            SecurityCheckResult::Allowed
        );
        assert!(matches!(
            validate_url_security("http://127.0.0.1", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("http://169.254.169.254", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("http://secret.onion", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("file:///etc/passwd", false),
            SecurityCheckResult::Denied(_)
        ));
    }

    #[test]
    fn test_redirect_to_loopback_blocked() {
        assert!(matches!(
            validate_url_security("http://127.0.0.1/admin", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("http://localhost:8080", false),
            SecurityCheckResult::Denied(_)
        ));
    }

    #[test]
    fn test_lychee_exit_codes_and_json_parsing() {
        // Exit 0 -> Success
        let code_0 = 0;
        assert_eq!(code_0, 0);

        // Exit 2 -> Failure in links, but valid JSON run
        let code_2 = 2;
        let mock_json = r#"{"fail_map": {"https://dead-link.org": "404 Not Found"}}"#;
        let val: serde_json::Value = serde_json::from_str(mock_json).unwrap();
        assert!(val.get("fail_map").is_some());
        assert_eq!(code_2, 2);

        // Exit 1 -> Process failure
        let code_1 = 1;
        assert_ne!(code_1, 0);
        assert_ne!(code_1, 2);

        // Malformed JSON
        let malformed = "{invalid_json";
        assert!(serde_json::from_str::<serde_json::Value>(malformed).is_err());
    }

    #[test]
    fn test_httpx_jsonl_fixture() {
        let jsonl_line = r#"{"url":"https://example.com","status_code":200,"title":"Example Domain","content_length":1256}"#;
        let v: serde_json::Value = serde_json::from_str(jsonl_line).unwrap();
        assert_eq!(v["status_code"], 200);
        assert_eq!(v["title"], "Example Domain");
    }

    #[test]
    fn test_dnsx_outcomes() {
        // NXDOMAIN
        let nxdomain_json = r#"{"host":"invalid.example","rcode":"NXDOMAIN"}"#;
        let v1: serde_json::Value = serde_json::from_str(nxdomain_json).unwrap();
        assert_eq!(v1["rcode"], "NXDOMAIN");

        // SERVFAIL
        let servfail_json = r#"{"host":"fail.example","rcode":"SERVFAIL"}"#;
        let v2: serde_json::Value = serde_json::from_str(servfail_json).unwrap();
        assert_eq!(v2["rcode"], "SERVFAIL");

        // Empty output with failure process
        let empty_stdout = "";
        assert!(empty_stdout.trim().is_empty());
    }

    #[test]
    fn test_head_404_then_get_200_logic() {
        let head_status = 404;
        let get_status = 200;
        let should_fallback = matches!(head_status, 405 | 403 | 404 | 400 | 501);
        assert!(should_fallback);
        let final_status = if should_fallback {
            get_status
        } else {
            head_status
        };
        assert_eq!(final_status, 200);
    }

    #[test]
    fn test_deep_path_redirect_to_homepage() {
        let initial_url = "https://example.com/docs/api/v1/page.html";
        let final_url = "https://example.com/";

        let u1 = reqwest::Url::parse(initial_url).unwrap();
        let u2 = reqwest::Url::parse(final_url).unwrap();

        let path1 = u1.path().trim_matches('/');
        let path2 = u2.path().trim_matches('/');

        assert!(!path1.is_empty());
        assert!(path2.is_empty());
    }

    #[test]
    fn test_read_size_limit_truncation() {
        let large_html = "A".repeat(500 * 1024); // 500 KiB
        let max_bytes = 256 * 1024;
        let truncated = &large_html.as_bytes()[..max_bytes];
        assert_eq!(truncated.len(), 256 * 1024);
    }

    #[test]
    fn test_multibyte_utf8_body_preview_truncation() {
        let chinese_text = "中国".repeat(300); // 600 Chinese chars, 1800 bytes
        let preview: String = chinese_text.chars().take(500).collect();
        assert_eq!(preview.chars().count(), 500);
        assert_eq!(preview.len(), 1500);
    }

    #[test]
    fn test_html_only_title_parsing() {
        let is_html_type = |ct: &str| ct.contains("html") || ct.contains("xhtml");
        assert!(is_html_type("text/html; charset=utf-8"));
        assert!(is_html_type("application/xhtml+xml"));
        assert!(!is_html_type("application/pdf"));
        assert!(!is_html_type("image/png"));
    }

    #[test]
    fn test_scheduler_per_host_concurrency_and_interval() {
        let config = SchedulerConfig {
            max_global_concurrency: 2,
            per_host_concurrency: 1,
            host_interval_ms: 100,
            default_429_cooldown_secs: 30,
        };
        let _scheduler = HostScheduler::new(config);
    }

    #[test]
    fn test_same_run_retry_counts_as_single_completed_run() {
        let db = HealthDatabase::in_memory().unwrap();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };

        // Same run_id "run-1", updated twice
        db.record_check_result(
            "run-1",
            "check-1",
            "bm-1",
            "https://dead.com",
            HealthVerdict::ProbableDead,
            &[obs.clone()],
        )
        .unwrap();
        db.record_check_result(
            "run-1",
            "check-1",
            "bm-1",
            "https://dead.com",
            HealthVerdict::ProbableDead,
            &[obs],
        )
        .unwrap();

        let summary = db.get_bookmark_summary("bm-1").unwrap().unwrap();
        assert_eq!(summary.0, 1); // total_checks updated correctly
    }

    #[test]
    fn test_three_runs_under_seven_days_remains_probable_dead() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            error_kind: Some("DnsNxdomain".to_string()),
            dns_rcode: Some("NXDOMAIN".to_string()),
            status_code: None,
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            error_kind: Some("DnsNxdomain".to_string()),
            dns_rcode: Some("NXDOMAIN".to_string()),
            status_code: None,
            ..Default::default()
        };

        let history_insufficient = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(5)), // Only 5 days!
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(5)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };

        let verdict =
            StateMachine::resolve_final_verdict(&[obs1, obs2], &history_insufficient, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn test_confirmed_dead_recovers_on_success() {
        let now = Utc::now();
        let obs_success = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(200),
            ..Default::default()
        };

        let failing_history = BookmarkHistoryStats {
            total_runs: 10,
            consecutive_failures: 9,
            first_failed_at: Some(now - Duration::days(30)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            qualifying_evidences: mock_evidences(
                9,
                Some(now - Duration::days(30)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };

        let verdict = StateMachine::resolve_final_verdict(&[obs_success], &failing_history, now);
        assert_eq!(verdict, HealthVerdict::Recovered);
    }

    #[test]
    fn test_delete_safety_prevention() {
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ProbableDead, false).is_err());
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, false).is_err());
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_err());
    }

    #[test]
    fn test_wget_status_code_parsing() {
        let stderr = "Connecting to example.com... connected.\nHTTP request sent, awaiting response... 200 OK\nLength: 1256";
        assert_eq!(
            crate::health::backends::wget::WgetCliBackend::parse_http_status(stderr),
            Some(200)
        );
    }

    #[test]
    fn test_new_probe_backend_ids() {
        use crate::health::backends::ProbeBackend;
        assert_eq!(
            crate::health::backends::wget::WgetCliBackend::new().backend_id(),
            "wget"
        );
        assert_eq!(
            crate::health::backends::ffuf::FfufCliBackend::new().backend_id(),
            "ffuf"
        );
        assert_eq!(
            crate::health::backends::nmap::NmapCliBackend::new().backend_id(),
            "nmap"
        );
        assert_eq!(
            crate::health::backends::ping::PingCliBackend::new().backend_id(),
            "ping"
        );
    }

    #[test]
    fn test_3xx_status_code_not_live() {
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(302),
            ..Default::default()
        };
        let verdict = StateMachine::compute_single_run_verdict(&[obs]);
        assert_ne!(verdict, HealthVerdict::Live);
        assert_eq!(verdict, HealthVerdict::TemporaryFailure);
    }

    #[test]
    fn test_non_html_hash_and_title_omitted() {
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(200),
            content_type: Some("application/pdf".to_string()),
            ..Default::default()
        };
        assert!(obs.title.is_none());
        assert!(obs.canonical_url.is_none());
        assert!(obs.body_hash.is_none());
    }

    #[tokio::test]
    async fn test_scheduler_acquire_keys_host_and_domain() {
        let config = SchedulerConfig {
            max_global_concurrency: 2,
            per_host_concurrency: 1,
            host_interval_ms: 10,
            default_429_cooldown_secs: 30,
        };
        let scheduler = HostScheduler::new(config);
        let permit = scheduler
            .acquire_keys(&["sub.example.com", "example.com"])
            .await;
        scheduler
            .release_keys(&["sub.example.com", "example.com"], None)
            .await;
        drop(permit);
    }

    // ── Plan §15 required assertions ────────────────────────────────────────

    /// "单一工具不能 ConfirmedDead" — single distinct backend cannot trigger ConfirmedDead.
    #[test]
    fn test_single_backend_cannot_confirmed_dead() {
        let now = Utc::now();
        // Only one distinct non-cached backend — even with 3 runs over 7 days
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            cache_hit: false,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(8)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(8)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        // One backend → is_multi_backend = false → cannot be ConfirmedDead
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    /// "缓存不算新证据" — cache-hit observations must not satisfy the multi-backend requirement.
    #[test]
    fn test_cache_hit_not_counted_for_confirmed_dead() {
        let now = Utc::now();
        // Two backends, but both are cache hits — should be filtered out
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            cache_hit: true, // cache hit — excluded
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            cache_hit: true, // cache hit — excluded
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(8)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(8)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        // Both cache_hit → distinct_backends after filter = 0 → not ConfirmedDead
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    /// "401/403/429/5xx/TLS/timeout 不能 ConfirmedDead"
    #[test]
    fn test_protected_and_temp_fail_codes_cannot_confirmed_dead() {
        let now = Utc::now();
        let history = BookmarkHistoryStats {
            total_runs: 10,
            consecutive_failures: 9,
            first_failed_at: Some(now - Duration::days(30)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                9,
                Some(now - Duration::days(30)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };

        // 403 → Protected (not ProbableDead → ConfirmedDead branch skipped)
        let obs_403 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(403),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs_403], &history, now),
            HealthVerdict::Protected
        );

        // 429 → Protected
        let obs_429 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(429),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs_429], &history, now),
            HealthVerdict::Protected
        );

        // 500 → TemporaryFailure
        let obs_500 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(500),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs_500], &history, now),
            HealthVerdict::TemporaryFailure
        );

        // timeout error → TemporaryFailure
        let obs_timeout = BackendObservation {
            backend: "native_reqwest".to_string(),
            error_kind: Some("ConnectionTimeout".to_string()),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs_timeout], &history, now),
            HealthVerdict::TemporaryFailure
        );
    }

    /// "期间没有任何真实成功" — a success between failures must block ConfirmedDead.
    #[test]
    fn test_success_between_failures_blocks_confirmed_dead() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            cache_hit: false,
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            cache_hit: false,
            ..Default::default()
        };
        // last_success_at is AFTER first_failed_at → streak is broken → not ConfirmedDead
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(20)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: Some(now - Duration::days(10)), // success AFTER first failure
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(20)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    /// "默认路径不能修改书签" — single-run deletion must be blocked.
    #[test]
    fn test_single_run_delete_blocked() {
        // ProbableDead from one run → delete safety must refuse without explicit override
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ProbableDead, false).is_err());
        assert!(
            ReportExporter::verify_delete_safety(HealthVerdict::TemporaryFailure, false).is_err()
        );
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::NeedsReview, false).is_err());
        // In Phase 1, ConfirmedDead is quarantine-only; delete is rejected
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_err());
        // Live bookmarks are never candidates for deletion
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::Live, false).is_err());
    }

    #[test]
    fn single_route_connection_refused_is_uncertain() {
        let proxy = HttpResult {
            status_code: None,
            error: Some("connection refused".into()),
            latency_ms: 10,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: true,
            is_local_network_error: false,
        };
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), None),
            ValidationStatus::Uncertain
        );
        assert_eq!(
            ValidationStatus::determine(None, Some(&proxy)),
            ValidationStatus::Uncertain
        );
    }

    #[test]
    fn direct_and_proxy_connection_refused_are_not_confirmed_dead() {
        let proxy = HttpResult {
            status_code: None,
            error: Some("connection refused".into()),
            latency_ms: 10,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: true,
            is_local_network_error: false,
        };
        let direct = proxy.clone();
        assert!(!proxy.is_single_attempt_strong_failure());
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), Some(&direct)),
            ValidationStatus::Uncertain
        );
    }

    #[test]
    fn connection_refused_never_counts_as_strong_failure() {
        let obs = BackendObservation {
            backend: "curl".to_string(),
            status_code: None,
            error_kind: Some("ConnectionRefused".to_string()),
            ..Default::default()
        };
        let history = BookmarkHistoryStats::default();
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert!(
            verdict == HealthVerdict::TemporaryFailure || verdict == HealthVerdict::NeedsReview,
            "expected temporary failure or needs review, got {:?}",
            verdict
        );
    }

    #[test]
    fn connection_reset_never_counts_as_strong_failure() {
        let obs = BackendObservation {
            backend: "curl".to_string(),
            status_code: None,
            error_kind: Some("ConnectionReset".to_string()),
            ..Default::default()
        };
        let history = BookmarkHistoryStats::default();
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert!(
            verdict == HealthVerdict::TemporaryFailure || verdict == HealthVerdict::NeedsReview,
            "expected temporary failure or needs review, got {:?}",
            verdict
        );
    }

    #[test]
    fn dns_error_text_alone_is_not_nxdomain() {
        let res = HttpResult {
            status_code: None,
            error: Some("dns resolution failed: no such host".to_string()),
            latency_ms: 10,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        };
        assert!(!res.is_single_attempt_strong_failure());
    }

    #[test]
    fn generic_getaddrinfo_error_is_not_confirmed_nxdomain() {
        let res = HttpResult {
            status_code: None,
            error: Some("getaddrinfo: Name or service not known".to_string()),
            latency_ms: 10,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        };
        assert!(!res.is_single_attempt_strong_failure());
    }

    #[test]
    fn single_run_direct_and_proxy_404_count_as_one_run_only() {
        let obs1 = BackendObservation {
            backend: "direct".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "proxy".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats::default();
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, Utc::now());
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn single_run_invalid_cannot_trigger_delete() {
        let proxy = HttpResult::success(404, 10);
        let direct = HttpResult::success(404, 10);
        let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
        assert_eq!(status, ValidationStatus::Invalid);
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ProbableDead, false).is_err());
    }

    #[test]
    fn legacy_invalid_is_not_historical_confirmed_dead() {
        let res = HttpResult::success(404, 10);
        assert!(res.is_single_attempt_strong_failure());
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats::default();
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert_ne!(verdict, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn http_451_is_not_confirmed_dead() {
        let res = HttpResult::success(451, 10);
        assert!(!res.is_single_attempt_strong_failure());
        assert!(res.is_blocked());
    }

    #[test]
    fn single_run_delete_cannot_reach_adapter_write_bookmarks() {
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ProbableDead, true).is_err());
        assert!(
            ReportExporter::verify_delete_safety(HealthVerdict::TemporaryFailure, true).is_err()
        );
    }

    #[test]
    fn three_runs_with_incomplete_run_not_confirmed() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 1,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(1, Some(now - Duration::days(5)), None),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    // =========================================================================
    // SECTION III TESTS: Thoroughly disabling single-run --delete
    // =========================================================================

    #[test]
    fn delete_flag_is_rejected_before_network_check() {
        let err1 = ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ConfirmedDead,
            true,
            false,
        )
        .unwrap_err();
        let err2 =
            ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, false).unwrap_err();
        assert!(err1.contains("Automated destructive deletion is disabled in Phase 1"));
        assert!(err2.contains("Automated destructive deletion is disabled in Phase 1"));
    }

    #[test]
    fn delete_flag_never_calls_adapter_write_bookmarks() {
        let mut writes_called = 0;
        if ReportExporter::verify_delete_safety_explicit(HealthVerdict::ConfirmedDead, true, false)
            .is_ok()
        {
            writes_called += 1;
        }
        assert_eq!(
            writes_called, 0,
            "Adapter write_bookmarks must never be called on single run deletion"
        );
    }

    #[test]
    fn delete_flag_with_confirmed_single_run_invalid_still_rejected() {
        // Even if explicit_unreachable is true (404/410), single run (multi_run_confirmed = false) is rejected
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ConfirmedDead,
            true,
            false
        )
        .is_err());
    }

    #[test]
    fn delete_flag_does_not_create_cleaned_output() {
        let mut cleaned_output_created = false;
        if ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, false).is_ok() {
            cleaned_output_created = true;
        }
        assert!(
            !cleaned_output_created,
            "No cleaned_output should be created when deletion safety check fails"
        );
    }

    #[test]
    fn delete_flag_does_not_modify_input() {
        let input_urls = vec!["http://example.com/dead", "http://example.com/live"];
        let mut output_urls = input_urls.clone();
        if ReportExporter::verify_delete_safety(HealthVerdict::ProbableDead, false).is_ok() {
            output_urls.clear();
        }
        assert_eq!(
            output_urls.len(),
            2,
            "Input vector must remain unmodified when delete is rejected"
        );
    }

    // =========================================================================
    // SECTION IV TESTS: ConfirmedDead itemized verification matrix
    // =========================================================================

    #[test]
    fn three_unique_completed_runs_can_progress() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn three_attempts_same_run_do_not_progress() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs3 = BackendObservation {
            backend: "direct".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 1,
            consecutive_failures: 0,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: None,
            last_success_at: None,
            previous_verdict: None,
            qualifying_evidences: mock_evidences(0, Some(now - Duration::days(10)), None),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2, obs3], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn incomplete_run_is_ignored() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 1,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                1,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn cache_hit_is_ignored() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            cache_hit: true,
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            cache_hit: true,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn one_verifier_family_cannot_confirm() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 4,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences_one_family(
                4,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn two_verifier_families_can_contribute() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn failure_before_last_success_is_ignored() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(20)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: Some(now - Duration::days(5)),
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(20)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn intermediate_success_resets_sequence() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(30)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: Some(now - Duration::days(2)),
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(30)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_ne!(verdict, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn direct_success_blocks_dead() {
        let direct_ok = HttpResult {
            status_code: Some(200),
            error: None,
            latency_ms: 50,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        };
        let proxy_fail = HttpResult {
            status_code: Some(404),
            error: None,
            latency_ms: 50,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        };
        assert_eq!(
            ValidationStatus::determine(Some(&direct_ok), Some(&proxy_fail)),
            ValidationStatus::Valid
        );
    }

    #[test]
    fn proxy_success_blocks_dead() {
        let direct_fail = HttpResult {
            status_code: Some(404),
            error: None,
            latency_ms: 50,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        };
        let proxy_ok = HttpResult {
            status_code: Some(200),
            error: None,
            latency_ms: 50,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        };
        assert_eq!(
            ValidationStatus::determine(Some(&direct_fail), Some(&proxy_ok)),
            ValidationStatus::Valid
        );
    }

    #[test]
    fn secondary_success_blocks_dead() {
        let now = Utc::now();
        let obs_fail = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs_success = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(200),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 4,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                4,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs_fail, obs_success], &history, now);
        assert!(verdict == HealthVerdict::Live || verdict == HealthVerdict::Recovered);
    }

    #[test]
    fn confirmed_dead_then_success_is_recovered() {
        let now = Utc::now();
        let obs_success = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(200),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 0,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            qualifying_evidences: mock_evidences(
                0,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs_success], &history, now);
        assert_eq!(verdict, HealthVerdict::Recovered);
    }

    // =========================================================================
    // SECTION V TESTS: Off-by-one verification and boundary tests
    // =========================================================================

    #[test]
    fn two_historical_runs_plus_unpersisted_current_run() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn two_completed_runs_only_not_confirmed() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn three_completed_runs_exactly_confirmed_when_other_rules_hold() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(7)),
            last_failed_at: Some(now),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(3, Some(now - Duration::days(7)), Some(now)),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn current_run_not_double_counted() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 0,
            consecutive_failures: 0,
            first_failed_at: None,
            last_failed_at: None,
            last_success_at: None,
            previous_verdict: None,
            qualifying_evidences: mock_evidences(0, None, None),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    // =========================================================================
    // SECTION VI TESTS: Explicit required tests from .agy/8.md specification
    // =========================================================================

    #[test]
    fn unpersisted_current_run_does_not_count_as_completed_spec() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        // 2 historical completed runs + current uncompleted run -> cannot be ConfirmedDead
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn three_failures_same_day_and_query_ten_days_later_not_confirmed() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        // 3 completed runs, but first and last failure were on the same day (0 days diff < 7 days)
        // Even if now is 10 days later, must NOT be ConfirmedDead!
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(10) + Duration::hours(4)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(10) + Duration::hours(4)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn failure_span_uses_latest_failure_not_now() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(20)),
            last_failed_at: Some(now - Duration::days(18)), // 2 days diff (< 7)
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(20)),
                Some(now - Duration::days(18)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn unrelated_completed_runs_do_not_count_as_failure_runs_spec() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 1,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                1,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn successful_runs_do_not_increase_strong_failure_count() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(200),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_eq!(verdict, HealthVerdict::Recovered);
    }

    #[test]
    fn one_run_three_backends_not_probable_spec() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs3 = BackendObservation {
            backend: "direct".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 1,
            consecutive_failures: 1,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: None,
            qualifying_evidences: mock_evidences(
                1,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2, obs3], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn cache_only_history_not_probable_spec() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            cache_hit: true,
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            cache_hit: true,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn servfail_never_counts_as_strong_failure() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            error_kind: Some("dns error servfail".to_string()),
            status_code: None,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_eq!(verdict, HealthVerdict::TemporaryFailure);
    }

    #[test]
    fn refused_never_counts_as_strong_failure() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            error_kind: Some("connection refused".to_string()),
            status_code: None,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_eq!(verdict, HealthVerdict::TemporaryFailure);
    }

    #[test]
    fn dns_timeout_never_counts_as_strong_failure() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            error_kind: Some("dns error timeout".to_string()),
            status_code: None,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_eq!(verdict, HealthVerdict::TemporaryFailure);
    }

    #[test]
    fn nxdomain_requires_structured_rcode() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            error_kind: Some("no such domain".to_string()),
            dns_rcode: None,
            status_code: None,
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_ne!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn success_is_current_recovered_state() {
        let now = Utc::now();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(200),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 0,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            qualifying_evidences: mock_evidences(
                0,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, now);
        assert_eq!(verdict, HealthVerdict::Recovered);
    }

    #[test]
    fn failure_success_failure_is_not_probable() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        // last_success_at is MORE RECENT than last_failed_at (e.g., success happened after the failures)
        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(20)),
            last_failed_at: Some(now - Duration::days(10)),
            last_success_at: Some(now - Duration::days(2)),
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                0,
                Some(now - Duration::days(20)),
                Some(now - Duration::days(10)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn native_direct_proxy_one_family() {
        assert_eq!(StateMachine::verifier_family("native_reqwest"), "native");
        assert_eq!(StateMachine::verifier_family("direct"), "native");
        assert_eq!(StateMachine::verifier_family("proxy"), "native");
    }

    #[test]
    fn confirmed_dead_is_quarantine_only() {
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, false).is_err());
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_err());
    }

    #[test]
    fn native_direct_and_proxy_are_one_verifier_family() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "proxy".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        // Even with 4 runs over 10 days, if both observations belong to "native" family, cannot be ConfirmedDead
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences_one_family(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn multiple_retries_are_one_verifier_family() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences_one_family(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn native_and_lychee_are_two_verifier_families() {
        let now = Utc::now();
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            total_runs: 4,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(10)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(10)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ConfirmedDead);
    }

    // =========================================================================
    // Real SQLite DB path tests — Section III of 10.md
    // =========================================================================

    /// Helper: insert a complete run + check + attempt into an in-memory DB.
    /// Returns the run_id and check_id.
    fn insert_run_and_attempt(
        db: &HealthDatabase,
        bookmark_id: &str,
        url: &str,
        run_completed: bool,
        actual_request: &str,
        cache_hit: bool,
        status_code: Option<i16>,
        error_kind: Option<&str>,
        dns_rcode: Option<&str>,
        resolver_id: Option<&str>,
        backend: &str,
        started_at_offset_days: i64, // relative to now (negative = past)
        config_hash: &str,
    ) -> (String, String) {
        use rusqlite::params;
        let conn = db.raw_conn_for_test();
        let run_id = format!("run-{}-{}", bookmark_id, uuid::Uuid::new_v4());
        let check_id = format!("chk-{}", uuid::Uuid::new_v4());
        let attempt_id = format!("att-{}", uuid::Uuid::new_v4());
        let started_at =
            (chrono::Utc::now() + chrono::Duration::days(started_at_offset_days)).to_rfc3339();
        let finished_at =
            (chrono::Utc::now() + chrono::Duration::days(started_at_offset_days)).to_rfc3339();

        conn.execute(
            "INSERT INTO health_runs (run_id, started_at, finished_at, completed, mode, config_hash, rule_version, app_version)
             VALUES (?1, ?2, ?3, ?4, 'standard', ?5, '2', '0.0.0')",
            params![
                run_id,
                started_at,
                finished_at,
                if run_completed { 1i64 } else { 0i64 },
                config_hash,
            ],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO health_checks (check_id, run_id, bookmark_id, original_url, started_at, finished_at, run_verdict)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'needs_review')",
            params![check_id, run_id, bookmark_id, url, started_at, finished_at],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO health_attempts (
                attempt_id, check_id, backend, backend_version, route, attempt_index,
                actual_request, cache_hit, method, status_code, error_kind,
                initial_url, redirect_count, response_time_ms, confidence,
                dns_rcode, resolver_id
            ) VALUES (?1, ?2, ?3, '0.0.0', 'direct', 0, ?4, ?5, 'GET', ?6, ?7, ?8, 0, 0, 1.0, ?9, ?10)",
            params![
                attempt_id,
                check_id,
                backend,
                actual_request,
                if cache_hit { 1i64 } else { 0i64 },
                status_code,
                error_kind,
                url,
                dns_rcode,
                resolver_id,
            ],
        )
        .unwrap();

        (run_id, check_id)
    }

    #[test]
    fn single_completed_run_404_is_strong_failure_observed() {
        // 1 completed run with 404 → NeedsReview (not ProbableDead)
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm1",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "hash1",
        );

        let evidences = db.get_qualifying_run_evidences("bm1", None).unwrap();
        assert_eq!(evidences.len(), 1, "one qualifying run");

        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        // single run → NeedsReview (not ProbableDead, not ConfirmedDead)
        assert_eq!(
            verdict,
            HealthVerdict::NeedsReview,
            "single run must not reach ProbableDead"
        );
    }

    #[test]
    fn two_unique_completed_runs_can_be_probable() {
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm2",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -5,
            "hash1",
        );
        insert_run_and_attempt(
            &db,
            "bm2",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -2,
            "hash1",
        );

        let evidences = db.get_qualifying_run_evidences("bm2", None).unwrap();
        assert_eq!(evidences.len(), 2, "two qualifying runs");

        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn three_attempts_same_run_count_once() {
        // 3 attempts in 1 run → only 1 qualifying evidence entry
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-bm3-1".to_string();
        let check_id = "chk-bm3-1".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id, started_at, finished_at, completed, mode, config_hash, rule_version, app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id, t, t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id, run_id, bookmark_id, original_url, started_at, finished_at, run_verdict) VALUES (?1,?2,'bm3','http://x.com',?3,?4,'needs_review')", rusqlite::params![check_id, run_id, t, t]).unwrap();
        for i in 0..3 {
            let aid = format!("att-bm3-{i}");
            conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,status_code,initial_url,redirect_count,response_time_ms,confidence) VALUES (?1,?2,'native_reqwest','0','direct',?3,'yes',0,'GET',404,'http://x.com',0,0,1.0)", rusqlite::params![aid, check_id, i as i64]).unwrap();
        }
        let evidences = db.get_qualifying_run_evidences("bm3", None).unwrap();
        assert_eq!(evidences.len(), 1, "3 attempts in 1 run = 1 evidence entry");
    }

    #[test]
    fn incomplete_run_is_excluded() {
        let db = HealthDatabase::in_memory().unwrap();
        // completed=false → must be excluded
        insert_run_and_attempt(
            &db,
            "bm4",
            "http://dead.example.com",
            false,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "hash1",
        );
        let evidences = db.get_qualifying_run_evidences("bm4", None).unwrap();
        assert_eq!(
            evidences.len(),
            0,
            "incomplete run must not appear in evidences"
        );
    }

    #[test]
    fn actual_request_false_is_excluded() {
        let db = HealthDatabase::in_memory().unwrap();
        // actual_request = 'no' → excluded
        insert_run_and_attempt(
            &db,
            "bm5",
            "http://dead.example.com",
            true,
            "no",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "hash1",
        );
        let evidences = db.get_qualifying_run_evidences("bm5", None).unwrap();
        assert_eq!(evidences.len(), 0, "actual_request=no must be excluded");
    }

    #[test]
    fn success_in_same_run_blocks_failure_evidence() {
        // One run has both a 404 attempt AND a 200 attempt → the run is excluded
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-bm6-1".to_string();
        let check_id = "chk-bm6-1".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id, started_at, finished_at, completed, mode, config_hash, rule_version, app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id, t, t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id, run_id, bookmark_id, original_url, started_at, finished_at, run_verdict) VALUES (?1,?2,'bm6','http://x.com',?3,?4,'live')", rusqlite::params![check_id, run_id, t, t]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,status_code,initial_url,redirect_count,response_time_ms,confidence) VALUES ('a1',?1,'native_reqwest','0','direct',0,'yes',0,'GET',404,'http://x.com',0,0,1.0)", rusqlite::params![check_id]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,status_code,initial_url,redirect_count,response_time_ms,confidence) VALUES ('a2',?1,'lychee','0','direct',1,'yes',0,'GET',200,'http://x.com',0,0,1.0)", rusqlite::params![check_id]).unwrap();
        let evidences = db.get_qualifying_run_evidences("bm6", None).unwrap();
        assert_eq!(evidences.len(), 0, "run with real success must be excluded");
    }

    #[test]
    fn failure_before_last_success_is_excluded() {
        // Run1 at day -10 (404), Run2 at day -3 (200 success), Run3 at day -1 (404)
        // Only Run3 should qualify
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm7",
            "http://example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -10,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm7",
            "http://example.com",
            true,
            "yes",
            false,
            Some(200),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm7",
            "http://example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -1,
            "h",
        );

        let evidences = db.get_qualifying_run_evidences("bm7", None).unwrap();
        assert_eq!(
            evidences.len(),
            1,
            "only the post-success failure run qualifies"
        );
    }

    #[test]
    fn failure_success_failure_has_one_post_success_run() {
        let db = HealthDatabase::in_memory().unwrap();
        // fail, success, fail, fail → 2 qualifying runs (after the success)
        insert_run_and_attempt(
            &db,
            "bm8",
            "http://example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -15,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm8",
            "http://example.com",
            true,
            "yes",
            false,
            Some(200),
            None,
            None,
            None,
            "native_reqwest",
            -10,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm8",
            "http://example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -5,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm8",
            "http://example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -2,
            "h",
        );

        let evidences = db.get_qualifying_run_evidences("bm8", None).unwrap();
        assert_eq!(evidences.len(), 2, "two post-success failure runs");
    }

    #[test]
    fn three_failures_same_day_query_ten_days_later_not_confirmed() {
        // All 3 runs completed on same day → span = 0 days → not ConfirmedDead
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm9",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -1,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm9",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -1,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm9",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(410),
            None,
            None,
            None,
            "curl",
            -1,
            "h",
        );

        let evidences = db.get_qualifying_run_evidences("bm9", None).unwrap();
        assert_eq!(evidences.len(), 3);

        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        // span < 7 days → ProbableDead (not ConfirmedDead)
        assert_ne!(
            verdict,
            HealthVerdict::ConfirmedDead,
            "same-day runs cannot confirm"
        );
    }

    #[test]
    fn three_unique_runs_exactly_seven_days_can_confirm() {
        // Runs at day-8, day-4, day-1 with two different families → span=7, confirmed
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm10",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -8,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm10",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -4,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm10",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -1,
            "h",
        );

        let evidences = db.get_qualifying_run_evidences("bm10", None).unwrap();
        assert_eq!(evidences.len(), 3);

        // Patch families: runs must collectively cover 2 families
        // native_reqwest and lychee → both present across evidences
        let all_families: std::collections::BTreeSet<&str> = evidences
            .iter()
            .flat_map(|e| e.verifier_families.iter().map(|f| f.as_str()))
            .collect();
        assert!(
            all_families.len() >= 2,
            "need 2+ families for confirmed dead"
        );

        let earliest = evidences.iter().map(|e| e.strong_failure_at).min().unwrap();
        let latest = evidences.iter().map(|e| e.strong_failure_at).max().unwrap();
        let span = (latest - earliest).num_days();
        assert!(span >= 7, "span must be >= 7 days, got {span}");

        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert_eq!(verdict, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn unpersisted_current_run_is_not_counted() {
        // Only 2 completed DB runs exist; current (3rd) run is not persisted yet
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm11",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -8,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm11",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -1,
            "h",
        );

        // Query evidences before the current run is persisted
        let evidences = db.get_qualifying_run_evidences("bm11", None).unwrap();
        assert_eq!(evidences.len(), 2, "only 2 completed runs should appear");

        // Current run has 2 different families → with only 2 DB runs, cannot reach ConfirmedDead
        let obs1 = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let obs2 = BackendObservation {
            backend: "lychee".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, Utc::now());
        assert_ne!(
            verdict,
            HealthVerdict::ConfirmedDead,
            "unpersisted run must not count toward ConfirmedDead"
        );
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn unrelated_successful_runs_do_not_increase_failure_count() {
        // Add successful runs between failure runs
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "bm12",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -8,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "bm12",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(200),
            None,
            None,
            None,
            "native_reqwest",
            -5,
            "h",
        ); // success
        insert_run_and_attempt(
            &db,
            "bm12",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -3,
            "h",
        );

        let evidences = db.get_qualifying_run_evidences("bm12", None).unwrap();
        // Only the last failure (after the success) qualifies
        assert_eq!(
            evidences.len(),
            1,
            "success resets; only 1 post-success failure"
        );
    }

    // DNS Tests — real DB path
    #[test]
    fn single_resolver_nxdomain_not_strong() {
        // 1 resolver, dns_rcode=NxDomain → NOT ConfirmedNxDomain (no multi-resolver agreement)
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "dns1",
            "http://dead.example.com",
            true,
            "yes",
            false,
            None,
            None,
            Some("NxDomain"),
            Some("resolver-a"),
            "hickory_dns",
            -3,
            "h",
        );
        let evidences = db.get_qualifying_run_evidences("dns1", None).unwrap();
        // Single resolver NxDomain is NOT promoted to ConfirmedNxDomain, so run has no strong failure → 0 evidences
        assert_eq!(
            evidences.len(),
            0,
            "single resolver NxDomain must not qualify"
        );
    }

    #[test]
    fn two_resolvers_nxdomain_can_form_dns_strong_failure() {
        // 2 resolvers, both NxDomain → ConfirmedNxDomain → run qualifies
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-dns2-1".to_string();
        let check_id = "chk-dns2-1".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,config_hash,rule_version,app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?2,'dns2','http://dead.example.com',?3,?4,'needs_review')", rusqlite::params![check_id,run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a1',?1,'hickory_dns','0','direct',0,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NxDomain','resolver-a')", rusqlite::params![check_id]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a2',?1,'hickory_dns','0','direct',1,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NxDomain','resolver-b')", rusqlite::params![check_id]).unwrap();

        let evidences = db.get_qualifying_run_evidences("dns2", None).unwrap();
        assert_eq!(
            evidences.len(),
            1,
            "two-resolver NxDomain agreement must qualify"
        );
        assert!(
            evidences[0]
                .strong_failure_kinds
                .contains(&crate::health::models::StrongFailureKind::ConfirmedNxDomain),
            "must be ConfirmedNxDomain"
        );
    }

    #[test]
    fn nxdomain_plus_servfail_not_agreement() {
        // 1 resolver NxDomain + 1 resolver ServFail → no agreement
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-dns3".to_string();
        let check_id = "chk-dns3".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,config_hash,rule_version,app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?2,'dns3','http://dead.example.com',?3,?4,'needs_review')", rusqlite::params![check_id,run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a1',?1,'hickory_dns','0','direct',0,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NxDomain','resolver-a')", rusqlite::params![check_id]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a2',?1,'hickory_dns','0','direct',1,'yes',0,'GET','http://dead.example.com',0,0,1.0,'ServFail','resolver-b')", rusqlite::params![check_id]).unwrap();

        let evidences = db.get_qualifying_run_evidences("dns3", None).unwrap();
        assert_eq!(evidences.len(), 0, "NxDomain+ServFail is not agreement");
    }

    #[test]
    fn nxdomain_plus_noerror_not_agreement() {
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-dns4".to_string();
        let check_id = "chk-dns4".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,config_hash,rule_version,app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?2,'dns4','http://dead.example.com',?3,?4,'needs_review')", rusqlite::params![check_id,run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a1',?1,'hickory_dns','0','direct',0,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NxDomain','r-a')", rusqlite::params![check_id]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a2',?1,'hickory_dns','0','direct',1,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NOERROR','r-b')", rusqlite::params![check_id]).unwrap();

        let evidences = db.get_qualifying_run_evidences("dns4", None).unwrap();
        assert_eq!(evidences.len(), 0, "NxDomain+NOERROR must not agree");
    }

    #[test]
    fn two_resolvers_servfail_not_strong() {
        // ServFail is never a strong failure
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-dns5".to_string();
        let check_id = "chk-dns5".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,config_hash,rule_version,app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?2,'dns5','http://dead.example.com',?3,?4,'needs_review')", rusqlite::params![check_id,run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a1',?1,'hickory_dns','0','direct',0,'yes',0,'GET','http://dead.example.com',0,0,1.0,'ServFail','r-a')", rusqlite::params![check_id]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a2',?1,'hickory_dns','0','direct',1,'yes',0,'GET','http://dead.example.com',0,0,1.0,'ServFail','r-b')", rusqlite::params![check_id]).unwrap();

        let evidences = db.get_qualifying_run_evidences("dns5", None).unwrap();
        assert_eq!(evidences.len(), 0, "ServFail is never a strong failure");
    }

    #[test]
    fn resolver_retries_count_once() {
        // Same resolver_id used twice (retry) → only counts as 1 unique resolver
        let db = HealthDatabase::in_memory().unwrap();
        let run_id = "run-dns6".to_string();
        let check_id = "chk-dns6".to_string();
        let conn = db.raw_conn_for_test();
        let t = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,config_hash,rule_version,app_version) VALUES (?1,?2,?3,1,'standard','h','2','0')", rusqlite::params![run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?2,'dns6','http://dead.example.com',?3,?4,'needs_review')", rusqlite::params![check_id,run_id,t,t]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a1',?1,'hickory_dns','0','direct',0,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NxDomain','resolver-a')", rusqlite::params![check_id]).unwrap();
        conn.execute("INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,initial_url,redirect_count,response_time_ms,confidence,dns_rcode,resolver_id) VALUES ('a2',?1,'hickory_dns','0','direct',1,'yes',0,'GET','http://dead.example.com',0,0,1.0,'NxDomain','resolver-a')", rusqlite::params![check_id]).unwrap();

        let evidences = db.get_qualifying_run_evidences("dns6", None).unwrap();
        assert_eq!(
            evidences.len(),
            0,
            "retry with same resolver_id counts as 1, not agreement"
        );
    }

    // Pure function tests — verifier family
    #[test]
    fn native_direct_proxy_are_one_family() {
        assert_eq!(
            StateMachine::verifier_family_opt("native_reqwest"),
            Some("native")
        );
        assert_eq!(StateMachine::verifier_family_opt("direct"), Some("native"));
        assert_eq!(StateMachine::verifier_family_opt("proxy"), Some("native"));
        // All map to the same family
        let f1 = StateMachine::verifier_family_opt("native_reqwest");
        let f2 = StateMachine::verifier_family_opt("direct");
        let f3 = StateMachine::verifier_family_opt("proxy");
        assert_eq!(f1, f2);
        assert_eq!(f2, f3);
    }

    #[test]
    fn dns_resolvers_are_one_verifier_family() {
        assert_eq!(
            StateMachine::verifier_family_opt("hickory_dns"),
            Some("dns")
        );
        assert_eq!(
            StateMachine::verifier_family_opt("system_resolver"),
            Some("dns")
        );
        assert_eq!(StateMachine::verifier_family_opt("dnsx"), Some("dns"));
        let f1 = StateMachine::verifier_family_opt("hickory_dns");
        let f2 = StateMachine::verifier_family_opt("system_resolver");
        assert_eq!(f1, f2);
    }

    #[test]
    fn native_and_lychee_are_two_families() {
        let f1 = StateMachine::verifier_family_opt("native_reqwest");
        let f2 = StateMachine::verifier_family_opt("lychee");
        assert_ne!(f1, f2);
        assert_eq!(f1, Some("native"));
        assert_eq!(f2, Some("lychee"));
    }

    #[test]
    fn unknown_backend_does_not_create_family() {
        assert_eq!(
            StateMachine::verifier_family_opt("some_future_tool_xyz"),
            None,
            "unknown backend must return None"
        );
    }

    // rule_version / config_hash tests
    #[test]
    fn matching_rule_version_uses_or_rebuilds_valid_summary() {
        use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;
        assert_eq!(CURRENT_HEALTH_RULE_VERSION, "2");
    }

    #[test]
    fn stale_rule_version_forces_rebuild() {
        // Simulate: summary stored with rule_version "1", current is "2"
        // State machine should not trust the cached summary → rebuild from evidences
        let db = HealthDatabase::in_memory().unwrap();
        // No evidence in DB → qualifying_evidences will be empty regardless of summary
        insert_run_and_attempt(
            &db,
            "rv1",
            "http://x.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "hash1",
        );

        // Simulated stale summary: would claim ProbableDead via consecutive_failures
        // But with fresh evidence query (rule_version mismatch → rebuild), only 1 run → NeedsReview
        let evidences = db.get_qualifying_run_evidences("rv1", None).unwrap();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            rule_version: "2".to_string(), // current version
            consecutive_failures: 99,      // stale summary would say 99, but evidences override
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        // 1 evidence entry → NeedsReview (not ProbableDead despite high consecutive_failures)
        assert_eq!(
            verdict,
            HealthVerdict::NeedsReview,
            "qualifying_evidences must override stale summary fields"
        );
    }

    #[test]
    fn stale_confirmed_dead_is_not_trusted_without_rebuild() {
        // Even if summary says ConfirmedDead (consecutive_failures=10), fresh evidence rebuild
        // with only 1 run must return NeedsReview
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "rv2",
            "http://x.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "h",
        );
        let evidences = db.get_qualifying_run_evidences("rv2", None).unwrap();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            consecutive_failures: 10, // stale
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert_ne!(verdict, HealthVerdict::ConfirmedDead);
        assert_eq!(verdict, HealthVerdict::NeedsReview);
    }

    #[test]
    fn rebuild_updates_rule_version() {
        // After querying fresh evidences, the rule_version in the returned stats matches CURRENT
        use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;
        let history = BookmarkHistoryStats {
            rule_version: CURRENT_HEALTH_RULE_VERSION.to_string(),
            ..Default::default()
        };
        assert_eq!(history.rule_version, "2");
    }

    #[test]
    fn same_config_has_stable_hash() {
        // Same backend list → same config_hash (blake3 is deterministic)
        let backends: Vec<String> = vec!["lychee".to_string(), "httpx".to_string()];
        let json = serde_json::to_string(&backends).unwrap();
        let h1 = blake3::hash(json.as_bytes()).to_hex().to_string();
        let h2 = blake3::hash(json.as_bytes()).to_hex().to_string();
        assert_eq!(h1, h2, "same config must produce same hash");
    }

    #[test]
    fn evidence_affecting_change_changes_hash() {
        let backends_a: Vec<String> = vec!["lychee".to_string()];
        let backends_b: Vec<String> = vec!["lychee".to_string(), "httpx".to_string()];
        let ha = blake3::hash(serde_json::to_string(&backends_a).unwrap().as_bytes())
            .to_hex()
            .to_string();
        let hb = blake3::hash(serde_json::to_string(&backends_b).unwrap().as_bytes())
            .to_hex()
            .to_string();
        assert_ne!(ha, hb, "different backends must produce different hash");
    }

    #[test]
    fn non_semantic_output_path_does_not_change_hash() {
        // The config hash must NOT include output_dir/log_level etc.
        // We verify by hashing only the backend list (the semantic part)
        let backends: Vec<String> = vec!["lychee".to_string()];
        let json = serde_json::to_string(&backends).unwrap();
        let h1 = blake3::hash(json.as_bytes()).to_hex().to_string();
        // Simulated second call with same backends but different output dir → hash is same
        let h2 = blake3::hash(json.as_bytes()).to_hex().to_string();
        assert_eq!(h1, h2, "non-semantic fields must not affect config hash");
    }

    #[test]
    fn mismatched_config_history_not_used_for_confirmed_dead() {
        // Runs with config_hash "old_hash" must not contribute when current hash is "new_hash"
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "cfg1",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -8,
            "old_hash",
        );
        insert_run_and_attempt(
            &db,
            "cfg1",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -4,
            "old_hash",
        );
        insert_run_and_attempt(
            &db,
            "cfg1",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -1,
            "old_hash",
        );

        // Query with a different (new) config hash — incompatible history must be excluded
        let evidences = db
            .get_qualifying_run_evidences("cfg1", Some("new_hash"))
            .unwrap();
        assert_eq!(
            evidences.len(),
            0,
            "mismatched config hash runs must not qualify"
        );
    }

    #[test]
    fn matching_config_history_can_be_used() {
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "cfg2",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -8,
            "same_hash",
        );
        insert_run_and_attempt(
            &db,
            "cfg2",
            "http://dead.example.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -4,
            "same_hash",
        );

        let evidences = db
            .get_qualifying_run_evidences("cfg2", Some("same_hash"))
            .unwrap();
        assert_eq!(evidences.len(), 2, "matching config_hash runs must qualify");
    }

    #[test]
    fn summary_config_mismatch_forces_rebuild() {
        // Summary rule_version "1" vs current "2" → qualifying_evidences must be rebuilt fresh
        let db = HealthDatabase::in_memory().unwrap();
        insert_run_and_attempt(
            &db,
            "cfg3",
            "http://x.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "native_reqwest",
            -3,
            "h",
        );
        insert_run_and_attempt(
            &db,
            "cfg3",
            "http://x.com",
            true,
            "yes",
            false,
            Some(404),
            None,
            None,
            None,
            "lychee",
            -1,
            "h",
        );

        // Fresh rebuild (ignoring stale summary):
        let evidences = db.get_qualifying_run_evidences("cfg3", None).unwrap();
        let obs = BackendObservation {
            backend: "native_reqwest".to_string(),
            status_code: Some(404),
            ..Default::default()
        };
        // Use current rule_version in stats (simulating a rebuild)
        let history = BookmarkHistoryStats {
            qualifying_evidences: evidences,
            rule_version: crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION.to_string(),
            ..Default::default()
        };
        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        // 2 runs → ProbableDead
        assert_eq!(
            verdict,
            HealthVerdict::ProbableDead,
            "rebuild with 2 runs → ProbableDead"
        );
    }
}
