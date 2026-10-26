//! Cached DNS observations remain visible without manufacturing independent runs.

use super::wire_tests::{context, fixture_for_provider};
use crate::health::backends::ProbeBackend;
use crate::health::db::HealthDatabase;
use crate::health::models::{ConfigHashInput, HealthVerdict, ProbeTarget, StrongFailureKind};
use crate::health::state_machine::{StateMachine, CURRENT_HEALTH_RULE_VERSION};
use hickory_resolver::proto::op::ResponseCode;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::test]
async fn cached_dns_runs_do_not_multiply_qualifying_history(
) -> Result<(), Box<dyn std::error::Error>> {
    let cloudflare = fixture_for_provider(ResponseCode::NXDomain, 60, "cloudflare").await?;
    let alidns = fixture_for_provider(ResponseCode::NXDomain, 60, "alidns").await?;
    let fixtures = [&cloudflare, &alidns];
    let target = ProbeTarget::from_url("site:cached.example.com", "http://cached.example.com/")?;
    let db = HealthDatabase::in_memory()?;
    let fresh_context = context("fresh");
    let config_hash = ConfigHashInput::from_probe_context(&fresh_context).stable_hash()?;
    let expected_source = serde_json::json!({
        "run_id": fresh_context.run_id.as_str(),
        "check_id": fresh_context.check_id.as_str(),
        "config_hash": config_hash.as_str(),
        "rule_version": CURRENT_HEALTH_RULE_VERSION,
    });
    db.record_run_start(&fresh_context)?;

    let mut fresh_observations = Vec::new();
    for fixture in fixtures {
        let observation = tokio::time::timeout(
            Duration::from_secs(3),
            fixture.backend.probe(&target, &fresh_context),
        )
        .await??;
        assert_eq!(observation.actual_request, "yes");
        assert!(!observation.cache_hit);
        assert_eq!(observation.dns_rcode.as_deref(), Some("NXDOMAIN"));
        assert_eq!(
            serde_json::to_value(&observation)?.get("source"),
            Some(&expected_source),
            "fresh DNS evidence must identify the run, check, configuration and rules that obtained it"
        );
        fresh_observations.push(observation);
    }
    assert_ne!(
        fresh_observations[0].resolver_id,
        fresh_observations[1].resolver_id
    );
    let fresh_verdict = StateMachine::compute_single_run_verdict_for_scope(
        &fresh_observations,
        fresh_context.probe_scope,
    );
    assert_eq!(fresh_verdict, HealthVerdict::ProbableDead);
    db.record_check_observations(
        &fresh_context.run_id,
        &fresh_context.check_id,
        &target.bookmark_id,
        &target.original_url,
        fresh_verdict,
        &fresh_observations,
    )?;
    db.record_run_finish(&fresh_context.run_id)?;

    let fresh_evidence = db.get_qualifying_run_evidences(
        &target.bookmark_id,
        Some(&config_hash),
        CURRENT_HEALTH_RULE_VERSION,
    )?;
    assert_eq!(
        fresh_evidence.len(),
        1,
        "fresh dual-provider evidence must qualify"
    );
    assert_eq!(fresh_evidence[0].run_id, fresh_context.run_id);
    assert!(fresh_evidence[0]
        .strong_failure_kinds
        .contains(&StrongFailureKind::ConfirmedNxDomain));
    let initial_wire_counts = fixtures.map(|fixture| fixture.wire_count.load(Ordering::SeqCst));
    assert!(initial_wire_counts.into_iter().all(|count| count > 0));

    for check_id in ["reuse1", "reuse2"] {
        let reused_context = context(check_id);
        db.record_run_start(&reused_context)?;
        let mut reused_observations = Vec::new();
        for (fixture, original) in fixtures.into_iter().zip(&fresh_observations) {
            let observation = tokio::time::timeout(
                Duration::from_secs(3),
                fixture.backend.probe(&target, &reused_context),
            )
            .await??;
            assert_eq!(observation.actual_request, "no");
            assert!(observation.cache_hit);
            assert_eq!(observation.dns_rcode.as_deref(), Some("NXDOMAIN"));
            assert_eq!(observation.started_at, original.started_at);
            assert_eq!(observation.finished_at, original.finished_at);
            assert_eq!(observation.run_id, reused_context.run_id);
            assert_eq!(observation.check_id, reused_context.check_id);
            assert_eq!(
                serde_json::to_value(&observation)?.get("source"),
                Some(&expected_source),
                "cache reuse must retain the original evidence source, not the new batch membership"
            );
            reused_observations.push(observation);
        }
        let reused_verdict = StateMachine::compute_single_run_verdict_for_scope(
            &reused_observations,
            reused_context.probe_scope,
        );
        db.record_check_observations(
            &reused_context.run_id,
            &reused_context.check_id,
            &target.bookmark_id,
            &target.original_url,
            reused_verdict,
            &reused_observations,
        )?;
        db.record_run_finish(&reused_context.run_id)?;

        let evidence = db.get_qualifying_run_evidences(
            &target.bookmark_id,
            Some(&config_hash),
            CURRENT_HEALTH_RULE_VERSION,
        )?;
        assert_eq!(
            evidence.len(),
            1,
            "cached runs must not add independent failures"
        );
        assert_eq!(evidence[0].run_id, fresh_context.run_id);
        assert_eq!(
            evidence[0].strong_failure_at,
            fresh_evidence[0].strong_failure_at
        );
        assert_eq!(
            fixtures.map(|fixture| fixture.wire_count.load(Ordering::SeqCst)),
            initial_wire_counts,
            "later batch records must not require another DNS query"
        );
    }

    let history = db.get_url_history(&target.original_url)?;
    assert_eq!(
        history.len(),
        3,
        "raw history must retain all three completed runs"
    );
    for check_id in ["fresh", "reuse1", "reuse2"] {
        assert!(history
            .iter()
            .any(|(run_id, _, _)| run_id == &context(check_id).run_id));
    }
    Ok(())
}
