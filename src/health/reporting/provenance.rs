//! Final reported evidence, distinct from invocation-wide traffic accounting.

use super::{write_atomic, BackendObservation, HealthVerdict};
use crate::health::models::ObservationSource;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

/// Counts of final reported target/evidence rows, not physical network requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportProvenanceCounts {
    pub fresh_probe_targets: usize,
    pub reused_only_targets: usize,
    pub unprobed_targets: usize,
    pub total_observations: usize,
    pub fresh_request_observations: usize,
    pub reused_observations: usize,
    pub no_request_observations: usize,
    pub unknown_request_observations: usize,
}

#[derive(Clone, Copy)]
enum EvidenceKind {
    Fresh,
    Reused,
    NoRequest,
    Unknown,
}

fn evidence_kind(observation: &BackendObservation) -> EvidenceKind {
    if observation.cache_hit
        || observation.request_method == "REUSE"
        || observation.backend == "same_day_resume"
    {
        EvidenceKind::Reused
    } else {
        match observation.actual_request.as_str() {
            "yes" => EvidenceKind::Fresh,
            "no" => EvidenceKind::NoRequest,
            _ => EvidenceKind::Unknown,
        }
    }
}

impl ReportProvenanceCounts {
    pub(super) fn from_results(
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)],
    ) -> Self {
        let mut counts = Self::default();
        for (_, _, _, observations) in results {
            let mut fresh = false;
            let mut reused = false;
            for observation in observations {
                counts.total_observations += 1;
                match evidence_kind(observation) {
                    EvidenceKind::Fresh => {
                        fresh = true;
                        counts.fresh_request_observations += 1;
                    }
                    EvidenceKind::Reused => {
                        reused = true;
                        counts.reused_observations += 1;
                    }
                    EvidenceKind::NoRequest => counts.no_request_observations += 1,
                    EvidenceKind::Unknown => counts.unknown_request_observations += 1,
                }
            }
            if fresh {
                counts.fresh_probe_targets += 1;
            } else if reused {
                counts.reused_only_targets += 1;
            } else {
                counts.unprobed_targets += 1;
            }
        }
        counts
    }
}

#[derive(Serialize)]
struct ProvenanceReport<'a> {
    schema_version: &'static str,
    scan_id: &'a str,
    probe_scope: &'a str,
    generated_at: DateTime<Utc>,
    accounting_scope: &'static str,
    total_targets: usize,
    counts: &'a ReportProvenanceCounts,
    targets: Vec<TargetEvidence<'a>>,
}

#[derive(Serialize)]
struct TargetEvidence<'a> {
    bookmark_id: &'a str,
    url: &'a str,
    verdict: &'static str,
    observations: Vec<ObservationEvidence<'a>>,
}

// Preserve identifying metadata without exporting response bodies or local artifact paths.
#[derive(Serialize)]
struct ObservationEvidence<'a> {
    backend: &'a str,
    backend_version: &'a str,
    run_id: &'a str,
    check_id: &'a str,
    source: &'a Option<ObservationSource>,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    actual_request: &'a str,
    cache_hit: bool,
    request_method: &'a str,
    status_code: Option<u16>,
    error_kind: Option<&'a str>,
    dns_rcode: Option<&'a str>,
    resolver_id: Option<&'a str>,
}

pub(super) fn write(
    directory: &Path,
    scan_id: &str,
    scope: &str,
    counts: &ReportProvenanceCounts,
    results: &[(String, String, HealthVerdict, Vec<BackendObservation>)],
) -> io::Result<()> {
    let report = ProvenanceReport {
        schema_version: "health-provenance-v1",
        scan_id,
        probe_scope: scope,
        generated_at: Utc::now(),
        accounting_scope: "final_reported_evidence_not_total_traffic",
        total_targets: results.len(),
        counts,
        targets: results
            .iter()
            .map(|(id, url, verdict, observations)| TargetEvidence {
                bookmark_id: id,
                url,
                verdict: verdict.as_str(),
                observations: observations
                    .iter()
                    .map(|observation| ObservationEvidence {
                        backend: &observation.backend,
                        backend_version: &observation.backend_version,
                        run_id: &observation.run_id,
                        check_id: &observation.check_id,
                        source: &observation.source,
                        started_at: observation.started_at,
                        finished_at: observation.finished_at,
                        actual_request: &observation.actual_request,
                        cache_hit: observation.cache_hit,
                        request_method: &observation.request_method,
                        status_code: observation.status_code,
                        error_kind: observation.error_kind.as_deref(),
                        dns_rcode: observation.dns_rcode.as_deref(),
                        resolver_id: observation.resolver_id.as_deref(),
                    })
                    .collect(),
            })
            .collect(),
    };
    let json = serde_json::to_vec_pretty(&report)?;
    write_atomic(directory.join("health-provenance.json"), &json)?;
    write_atomic(
        directory.join(format!("{scope}-health-provenance.json")),
        &json,
    )
}
