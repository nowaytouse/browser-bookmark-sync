//! Multi-Run Historical State Machine for Health Verdicts

use crate::health::models::{BackendObservation, HealthVerdict, QualifyingRunEvidence};
use chrono::{DateTime, Utc};

/// Current rule version. Increment this when evidence semantics change.
pub const CURRENT_HEALTH_RULE_VERSION: &str = "4";

/// Historical summary stats used by state machine transition decisions.
/// `qualifying_evidences` is the authoritative source; aggregate fields are for display only.
#[derive(Debug, Clone, Default)]
pub struct BookmarkHistoryStats {
    pub total_runs: usize,
    pub consecutive_failures: usize,
    pub first_failed_at: Option<DateTime<Utc>>,
    pub last_failed_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    /// Consecutive completed runs with a real, non-cached 2xx and no semantic-missing signal.
    pub consecutive_successes: usize,
    pub previous_verdict: Option<HealthVerdict>,
    pub rule_version: String,
    pub config_hash: String,
    pub qualifying_evidences: Vec<QualifyingRunEvidence>,
}

pub struct StateMachine;

impl StateMachine {
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
        if observations.is_empty() {
            return HealthVerdict::NeedsReview;
        }

        let mut has_success = false;
        let mut has_protected = false;
        let mut has_temp_fail = false;
        let mut has_probable_dead = false;
        let mut has_missing_content = false;
        let mut has_suspicious_content = false;

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
                    if (200..300).contains(&code) {
                        has_success = true;
                        continue;
                    }
                }
                has_protected = true;
                continue;
            }

            if let Some(ref err) = obs.error_kind {
                if err == "RedirectedToHomepage" || err == "ContentMissing" {
                    if !obs.cache_hit {
                        has_missing_content = true;
                    }
                } else if err == "Soft404Candidate" {
                    if !obs.cache_hit {
                        has_suspicious_content = true;
                    }
                } else if err == "DnsNxdomain" {
                    if !obs.cache_hit {
                        has_probable_dead = true;
                    }
                } else {
                    has_temp_fail = true;
                }
            }

            if let Some(ref rcode) = obs.dns_rcode {
                if rcode.eq_ignore_ascii_case("NXDOMAIN") && !obs.cache_hit {
                    has_probable_dead = true;
                }
            }

            if let Some(code) = obs.status_code {
                if (200..300).contains(&code) {
                    has_success = true;
                } else if matches!(code, 401 | 403 | 407 | 423 | 429 | 451 | 503 | 520..=527) {
                    has_protected = true;
                } else if matches!(code, 404 | 410) {
                    if !obs.cache_hit {
                        has_probable_dead = true;
                    }
                } else if (500..600).contains(&code) || (300..400).contains(&code) || code == 408 {
                    has_temp_fail = true;
                }
            }
        }

        // A 200 response only proves transport success. A content-aware probe can still prove
        // that the requested deep resource was removed or replaced by an unavailable page.
        if has_missing_content {
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

    /// Evaluate final verdict taking multi-run history into account.
    ///
    /// Uses `history.qualifying_evidences` (from raw DB query) as the authoritative
    /// source. `history.consecutive_failures` and `history.last_failed_at` are retained
    /// for display / reporting only and MUST NOT be used for state transitions.
    pub fn resolve_final_verdict(
        current_obs: &[BackendObservation],
        history: &BookmarkHistoryStats,
        _now: DateTime<Utc>,
    ) -> HealthVerdict {
        let single_verdict = Self::compute_single_run_verdict(current_obs);

        // A previously doubtful/dead bookmark needs two independent completed successful runs.
        if single_verdict == HealthVerdict::Live {
            // Recovered iff:
            //   (a) previous_verdict was an explicitly failing/uncertain state, OR
            //   (b) raw qualifying_evidences is non-empty (there is real DB-backed failure history)
            //       under the current rule_version + config_hash
            //
            // Forbidden: do NOT use consecutive_failures, total_runs, or last_failed_at
            // as the decision input — those are stale summary counters.
            let had_prior_failure = matches!(
                history.previous_verdict,
                Some(
                    HealthVerdict::ProbableDead
                        | HealthVerdict::ConfirmedDead
                        | HealthVerdict::TemporaryFailure
                        | HealthVerdict::NeedsReview
                )
            );
            let has_raw_failure_evidence = !history.qualifying_evidences.is_empty();

            if had_prior_failure || has_raw_failure_evidence {
                return if history.consecutive_successes >= 2 {
                    HealthVerdict::Recovered
                } else {
                    HealthVerdict::NeedsReview
                };
            }
            return HealthVerdict::Live;
        }

        if single_verdict == HealthVerdict::ProbableDead {
            let evidences = &history.qualifying_evidences;

            // ConfirmedDead: 3+ unique qualifying completed runs, 7+ day span, 2+ verifier families
            if evidences.len() >= 3 {
                let earliest = evidences.iter().map(|e| e.strong_failure_at).min();
                let latest = evidences.iter().map(|e| e.strong_failure_at).max();
                let span_days = match (earliest, latest) {
                    (Some(e), Some(l)) => (l - e).num_days(),
                    _ => 0,
                };

                let all_families: std::collections::BTreeSet<&str> = evidences
                    .iter()
                    .flat_map(|e| e.verifier_families.iter().map(String::as_str))
                    .collect();
                let is_multi_family = all_families.len() >= 2;

                if span_days >= 7 && is_multi_family {
                    return HealthVerdict::ConfirmedDead;
                }
            }

            // ProbableDead: 2+ unique completed runs whose real strong-failure attempts
            // span at least 24 hours.
            if evidences.len() >= 2 {
                let earliest = evidences.iter().map(|e| e.strong_failure_at).min();
                let latest = evidences.iter().map(|e| e.strong_failure_at).max();
                if matches!((earliest, latest), (Some(first), Some(last)) if last - first >= chrono::Duration::hours(24))
                {
                    return HealthVerdict::ProbableDead;
                }
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
    fn test_confirmed_dead_requires_7_days_and_3_runs() {
        let mut obs1 = BackendObservation::default();
        obs1.backend = "native_reqwest".to_string();
        obs1.error_kind = Some("RedirectedToHomepage".to_string());

        let mut obs2 = BackendObservation::default();
        obs2.backend = "lychee".to_string();
        obs2.error_kind = Some("RedirectedToHomepage".to_string());

        let now = Utc::now();
        let history_insufficient = BookmarkHistoryStats {
            total_runs: 2,
            consecutive_failures: 2,
            first_failed_at: Some(now - Duration::days(2)),
            last_failed_at: Some(now - Duration::days(2)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(3)),
                Some(now - Duration::days(2)),
            ),
            ..Default::default()
        };

        // Insufficient days -> ProbableDead
        let v1 = StateMachine::resolve_final_verdict(
            &[obs1.clone(), obs2.clone()],
            &history_insufficient,
            now,
        );
        assert_eq!(v1, HealthVerdict::ProbableDead);

        let history_sufficient = BookmarkHistoryStats {
            total_runs: 3,
            consecutive_failures: 3,
            first_failed_at: Some(now - Duration::days(8)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                3,
                Some(now - Duration::days(8)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };

        // 3 runs + 7+ days span + multi-family -> ConfirmedDead
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
            first_failed_at: Some(now - Duration::days(1)),
            last_failed_at: Some(now - Duration::days(1)),
            last_success_at: None,
            previous_verdict: Some(HealthVerdict::ProbableDead),
            qualifying_evidences: mock_evidences(
                2,
                Some(now - Duration::days(2)),
                Some(now - Duration::days(1)),
            ),
            ..Default::default()
        };

        // Explicit 404 without 7 days and 3 completed runs cannot be promoted to ConfirmedDead
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
    fn test_recovery_on_success() {
        let mut obs = BackendObservation::default();
        obs.status_code = Some(200);

        let history = BookmarkHistoryStats {
            total_runs: 5,
            consecutive_failures: 4,
            first_failed_at: Some(Utc::now() - Duration::days(10)),
            last_failed_at: Some(Utc::now() - Duration::days(1)),
            last_success_at: None,
            consecutive_successes: 2,
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            qualifying_evidences: mock_evidences(
                4,
                Some(Utc::now() - Duration::days(10)),
                Some(Utc::now() - Duration::days(1)),
            ),
            ..Default::default()
        };

        let verdict = StateMachine::resolve_final_verdict(&[obs], &history, Utc::now());
        assert_eq!(verdict, HealthVerdict::Recovered);
    }

    #[test]
    fn one_success_after_failure_stays_in_review() {
        let obs = BackendObservation {
            status_code: Some(200),
            ..Default::default()
        };
        let history = BookmarkHistoryStats {
            previous_verdict: Some(HealthVerdict::ConfirmedDead),
            consecutive_successes: 1,
            ..Default::default()
        };

        assert_eq!(
            StateMachine::resolve_final_verdict(&[obs], &history, Utc::now()),
            HealthVerdict::NeedsReview
        );
    }
}
