//! Same-day multi-run state machine for health verdicts

use crate::health::models::{BackendObservation, HealthVerdict, QualifyingRunEvidence};
use crate::health::targets::ProbeScope;
use chrono::{DateTime, Duration, Utc};

/// Current rule version. Increment this when evidence semantics change.
pub const CURRENT_HEALTH_RULE_VERSION: &str = "14";

/// Failure evidence expires after one rolling day. This avoids calendar-timezone
/// edge cases while ensuring routine health decisions finish in one scan day.
const EVIDENCE_WINDOW: Duration = Duration::hours(24);

/// Historical summary stats used by state machine transition decisions.
/// `qualifying_evidences` is the authoritative source; aggregate fields are for display only.
#[derive(Debug, Clone, Default)]
pub struct BookmarkHistoryStats {
    pub total_runs: usize,
    pub consecutive_failures: usize,
    pub first_failed_at: Option<DateTime<Utc>>,
    pub last_failed_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub previous_verdict: Option<HealthVerdict>,
    pub rule_version: String,
    pub config_hash: String,
    pub qualifying_evidences: Vec<QualifyingRunEvidence>,
}

pub struct StateMachine;

impl StateMachine {
    pub(crate) fn is_current_evidence(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        let age = now.signed_duration_since(at);
        age >= Duration::zero() && age <= EVIDENCE_WINDOW
    }

    /// Map backend string to verifier family.
    /// Returns `None` for unknown backends — they do NOT contribute to family count.
    pub fn verifier_family_opt(backend: &str) -> Option<&'static str> {
        let lower = backend.to_lowercase();
        if lower.contains("reqwest")
            || lower.contains("native")
            || lower == "direct"
            || lower == "proxy"
        {
            Some("native")
        } else if lower.contains("lychee") {
            Some("lychee")
        } else if lower.contains("httpx") {
            Some("httpx")
        } else if lower.contains("curl") {
            Some("curl")
        } else if lower.contains("dns") || lower.contains("hickory") || lower.contains("resolver") {
            Some("dns")
        } else if lower.contains("headless")
            || lower.contains("playwright")
            || lower.contains("chromium")
        {
            Some("headless")
        } else {
            None
        }
    }

    /// Backwards-compatible alias that returns "other" for unknown backends.
    pub fn verifier_family(backend: &str) -> &'static str {
        Self::verifier_family_opt(backend).unwrap_or("other")
    }

    /// Check if observations represent an explicit 404/410 or impossible-to-access domain (NXDOMAIN / expired domain),
    /// while NOT being an exempt onion or special domain.
    pub fn is_explicit_unreachable_or_404(observations: &[BackendObservation]) -> bool {
        for obs in observations {
            if crate::health::content::is_exempt_special_domain(&obs.initial_url)
                || obs
                    .final_url
                    .as_ref()
                    .is_some_and(|u| crate::health::content::is_exempt_special_domain(u))
            {
                return false;
            }

            if let Some(code) = obs.status_code {
                if matches!(code, 404 | 410) {
                    return true;
                }
            }

            if let Some(ref rcode) = obs.dns_rcode {
                if rcode.eq_ignore_ascii_case("NXDOMAIN") {
                    return true;
                }
            }

            if let Some(ref err) = obs.error_kind {
                if err == "DnsNxdomain" {
                    return true;
                }
            }
        }
        false
    }

    /// Calculate current run's initial verdict from backend observations
    pub fn compute_single_run_verdict(observations: &[BackendObservation]) -> HealthVerdict {
        Self::compute_single_run_verdict_for_scope(observations, ProbeScope::Content)
    }

    /// Calculate a run verdict without mixing site existence with deep-content validity.
    pub fn compute_single_run_verdict_for_scope(
        observations: &[BackendObservation],
        scope: ProbeScope,
    ) -> HealthVerdict {
        if scope == ProbeScope::Site {
            return Self::compute_site_run_verdict(observations);
        }

        if observations.is_empty() {
            return HealthVerdict::NeedsReview;
        }

        let mut has_success = false;
        let mut has_protected = false;
        let mut has_temp_fail = false;
        let mut has_probable_dead = false;
        let mut has_missing_content = false;
        let mut has_suspicious_content = false;
        let mut has_structured_uncertainty = false;
        let mut has_content_success = false;

        for obs in observations {
            // Rule 0: Onion websites (.onion) and special internal domains are EXEMPT
            // and default to Protected unless they successfully return 200 OK.
            let is_exempt = crate::health::content::is_exempt_special_domain(&obs.initial_url)
                || obs
                    .final_url
                    .as_ref()
                    .is_some_and(|u| crate::health::content::is_exempt_special_domain(u))
                || obs.error_kind.as_deref() == Some("OnionDomainExempt");

            if is_exempt {
                if let Some(code) = obs.status_code {
                    if (200..300).contains(&code)
                        && obs.actual_request == "yes"
                        && !obs.cache_hit
                        && obs.error_kind.is_none()
                    {
                        has_success = true;
                        continue;
                    }
                }
                has_protected = true;
                continue;
            }

            if let Some(ref err) = obs.error_kind {
                if err.starts_with("SecurityBlocked:") {
                    has_protected = true;
                } else if err == "ContentMissing" {
                    if obs.actual_request == "yes" && !obs.cache_hit {
                        has_missing_content = true;
                    } else {
                        has_temp_fail = true;
                    }
                } else if err == "StructuredContentUncertain" {
                    has_structured_uncertainty = true;
                } else if matches!(
                    err.as_str(),
                    "Soft404Candidate"
                        | "ParkedDomainCandidate"
                        | "BodyTooLarge"
                        | "RedirectedToHomepage"
                ) {
                    if !obs.cache_hit {
                        has_suspicious_content = true;
                    }
                } else if err == "DnsNxdomain" {
                    if obs.actual_request == "yes" && !obs.cache_hit {
                        has_probable_dead = true;
                    }
                } else {
                    has_temp_fail = true;
                }
            }

            if let Some(ref rcode) = obs.dns_rcode {
                if rcode.eq_ignore_ascii_case("NXDOMAIN")
                    && obs.actual_request == "yes"
                    && !obs.cache_hit
                {
                    has_probable_dead = true;
                }
            }

            if let Some(code) = obs.status_code {
                if (200..300).contains(&code) {
                    let is_real_success =
                        obs.actual_request == "yes" && !obs.cache_hit && obs.error_kind.is_none();
                    if is_real_success {
                        has_success = true;
                    }
                    if is_real_success
                        && obs
                            .body_hash
                            .as_deref()
                            .is_some_and(|hash| !hash.is_empty())
                        && obs.content_length.is_some_and(|length| length > 0)
                    {
                        has_content_success = true;
                    }
                } else if matches!(code, 401 | 403 | 407 | 423 | 429 | 451 | 503 | 520..=527) {
                    has_protected = true;
                } else if matches!(code, 404 | 410) {
                    if obs.actual_request == "yes" && !obs.cache_hit && obs.error_kind.is_none() {
                        has_probable_dead = true;
                    } else if !obs.cache_hit {
                        has_temp_fail = true;
                    }
                } else if (500..600).contains(&code) || (300..400).contains(&code) || code == 408 {
                    has_temp_fail = true;
                }
            }
        }

        // A 200 response only proves transport success. A content-aware probe can still prove
        // that the requested deep resource was removed or replaced by an unavailable page.
        if has_content_success {
            HealthVerdict::Live
        } else if has_structured_uncertainty {
            // A conflicting login/private/unknown player state is not deletion evidence,
            // even when another verifier reports removal or earlier runs failed.
            HealthVerdict::NeedsReview
        } else if has_missing_content {
            HealthVerdict::ProbableDead
        } else if has_suspicious_content {
            HealthVerdict::NeedsReview
        } else if has_success {
            HealthVerdict::Live
        } else if has_protected {
            HealthVerdict::Protected
        } else if has_probable_dead {
            HealthVerdict::ProbableDead
        } else if has_temp_fail {
            HealthVerdict::TemporaryFailure
        } else {
            HealthVerdict::NeedsReview
        }
    }

    fn compute_site_run_verdict(observations: &[BackendObservation]) -> HealthVerdict {
        if observations.is_empty() {
            return HealthVerdict::NeedsReview;
        }

        let mut has_live_response = false;
        let mut has_protected = false;
        let mut has_nxdomain = false;
        let mut has_temporary_failure = false;

        for observation in observations {
            if observation.error_kind.as_deref() == Some("OnionDomainExempt")
                || crate::health::content::is_exempt_special_domain(&observation.initial_url)
                || observation
                    .final_url
                    .as_deref()
                    .is_some_and(crate::health::content::is_exempt_special_domain)
            {
                has_protected = true;
                continue;
            }

            if observation
                .error_kind
                .as_deref()
                .is_some_and(|error| error.starts_with("SecurityBlocked:"))
            {
                has_protected = true;
                continue;
            }

            if observation.actual_request == "yes" && !observation.cache_hit {
                if let Some(status) = observation.status_code {
                    match status {
                        401 | 403 | 407 | 423 | 429 | 451 => has_protected = true,
                        408 | 500..=599 => has_temporary_failure = true,
                        100..=499 => has_live_response = true,
                        _ => has_temporary_failure = true,
                    }
                }

                if observation
                    .dns_rcode
                    .as_deref()
                    .is_some_and(|rcode| rcode.eq_ignore_ascii_case("NXDOMAIN"))
                    || observation.error_kind.as_deref() == Some("DnsNxdomain")
                {
                    has_nxdomain = true;
                }
            } else if observation.error_kind.is_some() {
                has_temporary_failure = true;
            }
        }

        if has_live_response {
            HealthVerdict::Live
        } else if has_protected {
            HealthVerdict::Protected
        } else if has_nxdomain {
            HealthVerdict::ProbableDead
        } else if has_temporary_failure {
            HealthVerdict::TemporaryFailure
        } else {
            HealthVerdict::NeedsReview
        }
    }

    /// Evaluate final verdict taking multi-run history into account.
    ///
    /// Uses `history.qualifying_evidences` (from raw DB query) as the authoritative
    /// source. `history.consecutive_failures` and `history.last_failed_at` are retained
    /// for display / reporting only and MUST NOT be used for state transitions.
    pub fn resolve_final_verdict(
        current_obs: &[BackendObservation],
        history: &BookmarkHistoryStats,
        now: DateTime<Utc>,
    ) -> HealthVerdict {
        Self::resolve_final_verdict_for_scope(current_obs, history, now, ProbeScope::Content)
    }

    pub fn resolve_final_verdict_for_scope(
        current_obs: &[BackendObservation],
        history: &BookmarkHistoryStats,
        now: DateTime<Utc>,
        scope: ProbeScope,
    ) -> HealthVerdict {
        let single_verdict = Self::compute_single_run_verdict_for_scope(current_obs, scope);

        // A successful retry is enough to preserve the bookmark. Historical failures are
        // relevant only inside the same rolling-day evidence window.
        if single_verdict == HealthVerdict::Live {
            let had_prior_failure = matches!(
                history.previous_verdict,
                Some(
                    HealthVerdict::ProbableDead
                        | HealthVerdict::ConfirmedDead
                        | HealthVerdict::TemporaryFailure
                        | HealthVerdict::NeedsReview
                )
            );
            let has_current_failure = history
                .last_failed_at
                .is_some_and(|at| Self::is_current_evidence(at, now))
                || history
                    .qualifying_evidences
                    .iter()
                    .any(|evidence| Self::is_current_evidence(evidence.strong_failure_at, now));

            if had_prior_failure && has_current_failure {
                return HealthVerdict::Recovered;
            }
            return HealthVerdict::Live;
        }

        if single_verdict == HealthVerdict::ProbableDead {
            let mut evidences: Vec<_> = history
                .qualifying_evidences
                .iter()
                .filter(|evidence| Self::is_current_evidence(evidence.strong_failure_at, now))
                .filter(|evidence| {
                    scope == ProbeScope::Content
                        || evidence
                            .strong_failure_kinds
                            .contains(&crate::health::models::StrongFailureKind::ConfirmedNxDomain)
                })
                .collect();
            evidences.sort_by_key(|evidence| evidence.completed_at);

            // ConfirmedDead: three current-day completed runs and two verifier families.
            if evidences.len() >= 3 {
                let all_families: std::collections::BTreeSet<&str> = evidences
                    .iter()
                    .flat_map(|e| e.verifier_families.iter().map(String::as_str))
                    .collect();
                let is_multi_family = all_families.len() >= 2;
                let all_runs_confirmed_nxdomain = evidences.iter().all(|evidence| {
                    evidence
                        .strong_failure_kinds
                        .contains(&crate::health::models::StrongFailureKind::ConfirmedNxDomain)
                });
                let last_three_confirmed_content_missing =
                    evidences.iter().rev().take(3).all(|evidence| {
                        evidence
                            .strong_failure_kinds
                            .contains(&crate::health::models::StrongFailureKind::ContentMissing)
                    });

                if is_multi_family
                    || all_runs_confirmed_nxdomain
                    || last_three_confirmed_content_missing
                {
                    return HealthVerdict::ConfirmedDead;
                }
            }

            // Two current-day completed strong-failure runs are probable, regardless of
            // spacing. A later success immediately moves the bookmark back to the live set.
            if evidences.len() >= 2 {
                return HealthVerdict::ProbableDead;
            }

            // Fewer than 2 qualifying runs — not yet probable.
            return HealthVerdict::NeedsReview;
        }

        single_verdict
    }
}

#[cfg(test)]
mod tests {

    fn mock_evidences(
        count: usize,
        first: Option<DateTime<Utc>>,
        last: Option<DateTime<Utc>>,
    ) -> Vec<QualifyingRunEvidence> {
        use crate::health::models::{QualifyingRunEvidence, StrongFailureKind};
        use std::collections::BTreeSet;
        let mut evs = Vec::new();
        if count == 0 {
            return evs;
        }
        let first_dt = first.unwrap_or_else(Utc::now);
        let last_dt = last.unwrap_or_else(Utc::now);

        // Spread the dates
        let span = last_dt.signed_duration_since(first_dt).num_seconds();
        let step = if count > 1 {
            span / (count as i64 - 1)
        } else {
            0
        };

        for i in 0..count {
            let dt = first_dt + Duration::seconds(step * i as i64);
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

    fn mock_dns_evidences(
        count: usize,
        first: DateTime<Utc>,
        last: DateTime<Utc>,
    ) -> Vec<QualifyingRunEvidence> {
        use crate::health::models::StrongFailureKind;
        let mut evs = mock_evidences(count, Some(first), Some(last));
        for evidence in &mut evs {
            evidence.verifier_families.clear();
            evidence.verifier_families.insert("dns".to_string());
            evidence.strong_failure_kinds.clear();
            evidence
                .strong_failure_kinds
                .insert(StrongFailureKind::ConfirmedNxDomain);
        }
        evs
    }

    fn mock_content_missing_evidences(
        count: usize,
        first: DateTime<Utc>,
        last: DateTime<Utc>,
    ) -> Vec<QualifyingRunEvidence> {
        use crate::health::models::StrongFailureKind;
        let mut evidences = mock_evidences(count, Some(first), Some(last));
        for evidence in &mut evidences {
            evidence.verifier_families.clear();
            evidence.verifier_families.insert("native".to_string());
            evidence.strong_failure_kinds.clear();
            evidence
                .strong_failure_kinds
                .insert(StrongFailureKind::ContentMissing);
        }
        evidences
    }

    use super::*;
    use chrono::Duration;

    #[test]
    fn test_single_run_success() {
        let mut obs = BackendObservation::default();
        obs.status_code = Some(200);
        let verdict = StateMachine::compute_single_run_verdict(&[obs]);
        assert_eq!(verdict, HealthVerdict::Live);
    }

    #[test]
    fn site_scope_does_not_treat_page_failures_as_site_death() {
        for status in [302, 404, 410] {
            let observation = BackendObservation {
                actual_request: "yes".to_string(),
                status_code: Some(status),
                error_kind: (status == 404).then(|| "ContentMissing".to_string()),
                ..Default::default()
            };
            assert_eq!(
                StateMachine::compute_single_run_verdict_for_scope(
                    &[observation.clone()],
                    ProbeScope::Site,
                ),
                HealthVerdict::Live
            );
            if status == 404 {
                assert_eq!(
                    StateMachine::compute_single_run_verdict_for_scope(
                        &[observation],
                        ProbeScope::Content,
                    ),
                    HealthVerdict::ProbableDead
                );
            }
        }
    }

    #[test]
    fn site_scope_ignores_historical_page_failures() {
        let now = Utc::now();
        let observation = BackendObservation {
            actual_request: "yes".to_string(),
            dns_rcode: Some("NXDOMAIN".to_string()),
            error_kind: Some("DnsNxdomain".to_string()),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: mock_evidences(3, Some(now - Duration::hours(2)), Some(now)),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::resolve_final_verdict_for_scope(
                &[observation],
                &history,
                now,
                ProbeScope::Site,
            ),
            HealthVerdict::NeedsReview
        );
    }

    #[test]
    fn any_real_success_wins_over_a_protected_secondary_probe() {
        let mut success = BackendObservation::default();
        success.backend = "native_reqwest".to_string();
        success.actual_request = "yes".to_string();
        success.status_code = Some(200);

        let mut protected = BackendObservation::default();
        protected.backend = "curl".to_string();
        protected.actual_request = "yes".to_string();
        protected.status_code = Some(403);

        assert_eq!(
            StateMachine::compute_single_run_verdict(&[success, protected]),
            HealthVerdict::Live
        );
    }

    #[test]
    fn synthetic_or_security_blocked_2xx_is_not_success() {
        let mut observation = BackendObservation::default();
        observation.actual_request = "no".to_string();
        observation.status_code = Some(200);
        observation.error_kind = Some("SecurityBlocked: private address".to_string());
        assert_eq!(
            StateMachine::compute_single_run_verdict(&[observation]),
            HealthVerdict::Protected
        );
    }

    #[test]
    fn test_confirmed_dead_requires_three_same_day_runs() {
        let mut obs1 = BackendObservation::default();
        obs1.backend = "native_reqwest".to_string();
        obs1.status_code = Some(404);

        let mut obs2 = BackendObservation::default();
        obs2.backend = "lychee".to_string();
        obs2.status_code = Some(404);

        let now = Utc::now();
        let history_insufficient = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::minutes(2)),
            last_failed_at: Some(now - Duration::minutes(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::minutes(2)),
                Some(now - Duration::minutes(1)),
            ),
            ..Default::default()
        };

        // Two completed failures are probable, but not deletion-eligible.
        let v1 = StateMachine::resolve_final_verdict(
            &[obs1.clone(), obs2.clone()],
            &history_insufficient,
            now,
        );
        assert_eq!(v1, HealthVerdict::ProbableDead);

        let history_sufficient = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::minutes(3)),
            last_failed_at: Some(now - Duration::minutes(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::minutes(3)),
                Some(now - Duration::minutes(1)),
            ),
            ..Default::default()
        };

        // Three same-day runs plus multiple verifier families -> ConfirmedDead.
        let v2 = StateMachine::resolve_final_verdict(&[obs1, obs2], &history_sufficient, now);
        assert_eq!(v2, HealthVerdict::ConfirmedDead);
    }

    #[test]
    fn test_explicit_404_without_full_history_remains_probable_dead() {
        let mut obs1 = BackendObservation::default();
        obs1.backend = "native_reqwest".to_string();
        obs1.status_code = Some(404);

        let mut obs2 = BackendObservation::default();
        obs2.backend = "lychee".to_string();
        obs2.status_code = Some(404);

        let now = Utc::now();
        let history = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::minutes(2)),
            last_failed_at: Some(now - Duration::minutes(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::minutes(2)),
                Some(now - Duration::minutes(1)),
            ),
            ..Default::default()
        };

        // Two runs remain probable and cannot be deleted.
        let verdict = StateMachine::resolve_final_verdict(&[obs1, obs2], &history, now);
        assert_eq!(verdict, HealthVerdict::ProbableDead);
    }

    #[test]
    fn test_onion_domain_default_protected() {
        let mut obs = BackendObservation::default();
        obs.initial_url = "http://expyuz5wqqfdgah56...onion/test".to_string();
        obs.error_kind = Some("DnsNxdomain".to_string());
        obs.dns_rcode = Some("NXDOMAIN".to_string());

        let verdict = StateMachine::compute_single_run_verdict(&[obs]);
        assert_eq!(verdict, HealthVerdict::Protected);
    }

    #[test]
    fn security_blocked_is_protected_without_retry_eligible_failure() {
        let obs = BackendObservation {
            error_kind: Some("SecurityBlocked: private IP address".to_string()),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::compute_single_run_verdict(&[obs]),
            HealthVerdict::Protected
        );
    }

    #[test]
    fn three_same_day_dns_agreements_can_confirm_without_second_family() {
        let now = Utc::now();
        let obs = BackendObservation {
            error_kind: Some("DnsNxdomain".to_string()),
            dns_rcode: Some("NXDOMAIN".to_string()),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_dns_evidences(
                3,
                now - Duration::minutes(3),
                now - Duration::minutes(1),
            ),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs], &history, now),
            HealthVerdict::ConfirmedDead
        );
    }

    #[test]
    fn three_same_day_high_confidence_content_missing_runs_can_confirm_one_family() {
        let now = Utc::now();
        let evidences = mock_content_missing_evidences(
            3,
            now - Duration::minutes(3),
            now - Duration::minutes(1),
        );
        let observation = BackendObservation {
            status_code: Some(200),
            error_kind: Some("ContentMissing".to_string()),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: evidences,
            ..Default::default()
        };

        assert_eq!(
            StateMachine::resolve_final_verdict(&[observation], &history, now),
            HealthVerdict::ConfirmedDead
        );
    }

    #[test]
    fn content_aware_success_after_three_failures_is_recovered_not_deleted() {
        let now = Utc::now();
        let missing = BackendObservation {
            status_code: Some(200),
            error_kind: Some("ContentMissing".to_string()),
            body_hash: Some("missing".to_string()),
            ..Default::default()
        };
        let content_success = BackendObservation {
            status_code: Some(200),
            body_hash: Some("live-content".to_string()),
            content_length: Some(16),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ProbableDead),
            last_failed_at: Some(now - Duration::minutes(1)),
            qualifying_evidences: mock_content_missing_evidences(
                3,
                now - Duration::minutes(3),
                now - Duration::minutes(1),
            ),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::compute_single_run_verdict(&[missing.clone(), content_success.clone()]),
            HealthVerdict::Live
        );
        assert_eq!(
            StateMachine::resolve_final_verdict(&[missing, content_success], &history, now),
            HealthVerdict::Recovered
        );
    }

    #[test]
    fn header_only_or_empty_content_success_does_not_override_content_missing() {
        let missing = BackendObservation {
            status_code: Some(200),
            error_kind: Some("ContentMissing".to_string()),
            body_hash: Some("missing".to_string()),
            ..Default::default()
        };
        let header_only_success = BackendObservation {
            status_code: Some(200),
            request_method: "HEAD".to_string(),
            body_hash: None,
            ..Default::default()
        };
        let empty_hash_success = BackendObservation {
            status_code: Some(200),
            body_hash: Some(String::new()),
            content_length: Some(16),
            ..Default::default()
        };
        let empty_body_success = BackendObservation {
            status_code: Some(200),
            body_hash: Some("blake3-empty-body".to_string()),
            content_length: Some(0),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::compute_single_run_verdict(&[
                missing,
                header_only_success,
                empty_hash_success,
                empty_body_success,
            ]),
            HealthVerdict::ProbableDead
        );
    }

    #[test]
    fn real_content_success_precedes_a_protected_secondary_signal() {
        let protected = BackendObservation {
            status_code: Some(403),
            ..Default::default()
        };
        let content_success = BackendObservation {
            status_code: Some(200),
            body_hash: Some("live-content".to_string()),
            content_length: Some(16),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::compute_single_run_verdict(&[protected, content_success]),
            HealthVerdict::Live
        );
    }

    #[test]
    fn homepage_redirects_remain_review_only_despite_repeated_history() {
        use crate::health::models::StrongFailureKind;

        let now = Utc::now();
        let mut evidences = mock_evidences(
            3,
            Some(now - Duration::minutes(3)),
            Some(now - Duration::minutes(1)),
        );
        for evidence in &mut evidences {
            evidence.verifier_families.clear();
            evidence.verifier_families.insert("native".to_string());
            evidence.strong_failure_kinds.clear();
            evidence
                .strong_failure_kinds
                .insert(StrongFailureKind::RedirectedToHomepage);
        }
        let observation = BackendObservation {
            status_code: Some(200),
            error_kind: Some("RedirectedToHomepage".to_string()),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: evidences,
            ..Default::default()
        };

        assert_eq!(
            StateMachine::resolve_final_verdict(&[observation], &history, now),
            HealthVerdict::NeedsReview
        );
    }

    #[test]
    fn truncated_content_is_review_only_even_with_http_success() {
        let observation = BackendObservation {
            status_code: Some(200),
            error_kind: Some("BodyTooLarge".to_string()),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::compute_single_run_verdict(&[observation]),
            HealthVerdict::NeedsReview
        );
    }

    #[test]
    fn accuracy_incomplete_current_failures_cannot_borrow_confirmed_history() {
        let now = Utc::now();
        let history = BookmarkHistoryStats {
            qualifying_evidences: mock_evidences(3, Some(now - Duration::minutes(3)), Some(now)),
            ..Default::default()
        };
        for status in [404, 410] {
            for (actual, cached, error) in [
                ("unknown", false, Some("BodyReadError: incomplete message")),
                ("unknown", false, Some("ContentMissing")),
                ("no", false, None),
                ("yes", true, None),
                ("yes", false, Some("Timeout")),
            ] {
                let observation = BackendObservation {
                    status_code: Some(status),
                    actual_request: actual.into(),
                    cache_hit: cached,
                    error_kind: error.map(str::to_owned),
                    ..Default::default()
                };
                assert!(
                    !matches!(
                        StateMachine::resolve_final_verdict(&[observation], &history, now),
                        HealthVerdict::ProbableDead | HealthVerdict::ConfirmedDead
                    ),
                    "incomplete {status}: {actual}, cached={cached}, {error:?}"
                );
            }
            let complete = BackendObservation {
                status_code: Some(status),
                ..Default::default()
            };
            assert_eq!(
                StateMachine::resolve_final_verdict(&[complete], &history, now),
                HealthVerdict::ConfirmedDead
            );
        }
        for status in [
            301, 302, 401, 403, 407, 408, 423, 429, 451, 500, 502, 503, 520, 527,
        ] {
            let observation = BackendObservation {
                status_code: Some(status),
                ..Default::default()
            };
            assert!(!matches!(
                StateMachine::resolve_final_verdict(&[observation], &history, now),
                HealthVerdict::ProbableDead | HealthVerdict::ConfirmedDead
            ));
        }
    }

    #[test]
    fn accuracy_conflicting_player_state_cannot_borrow_deletion_history() {
        let now = Utc::now();
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            qualifying_evidences: mock_content_missing_evidences(
                3,
                now - Duration::minutes(3),
                now,
            ),
            ..Default::default()
        };
        let uncertain = BackendObservation {
            backend: "native-reqwest".into(),
            status_code: Some(200),
            error_kind: Some("StructuredContentUncertain".into()),
            ..Default::default()
        };
        let missing = BackendObservation {
            backend: "curl".into(),
            status_code: Some(200),
            error_kind: Some("ContentMissing".into()),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[missing.clone()], &history, now),
            HealthVerdict::ConfirmedDead
        );
        for observations in [
            vec![uncertain.clone(), missing.clone()],
            vec![missing, uncertain.clone()],
        ] {
            assert_eq!(
                StateMachine::resolve_final_verdict(&observations, &history, now),
                HealthVerdict::NeedsReview
            );
        }
        let complete_success = BackendObservation {
            status_code: Some(200),
            content_length: Some(100),
            body_hash: Some("valid-content".into()),
            ..Default::default()
        };
        assert_eq!(
            StateMachine::resolve_final_verdict(&[uncertain, complete_success], &history, now),
            HealthVerdict::Recovered
        );
    }

    #[test]
    fn test_recovery_on_success() {
        let mut obs = BackendObservation::default();
        obs.status_code = Some(200);

        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 4,
            first_failed_at: Some(Utc::now() - Duration::minutes(10)),
            last_failed_at: Some(Utc::now() - Duration::minutes(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            qualifying_evidences: mock_evidences(
                4,
                Some(Utc::now() - Duration::minutes(10)),
                Some(Utc::now() - Duration::minutes(1)),
            ),
            ..Default::default()
        };

        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert_eq!(verdict, HealthVerdict::Recovered);
    }

    #[test]
    fn one_success_after_failure_is_recovered() {
        let obs = BackendObservation {
            status_code: Some(200),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            last_failed_at: Some(Utc::now() - Duration::minutes(1)),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs], &history, Utc::now()),
            HealthVerdict::Recovered
        );
    }

    #[test]
    fn prior_day_failure_does_not_affect_today() {
        let now = Utc::now();
        let obs = BackendObservation {
            status_code: Some(404),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::hours(30)),
                Some(now - Duration::hours(25)),
            ),
            ..Default::default()
        };

        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs], &history, now),
            HealthVerdict::NeedsReview
        );
    }
}
