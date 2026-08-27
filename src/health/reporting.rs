//! Health Reports Generation and Delete Safety Enforcement

use crate::health::models::{BackendObservation, HealthVerdict};
use crate::health::targets::ProbeScope;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::create_dir_all;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthRunSummaryReport {
    pub run_id: String,
    pub probe_scope: String,
    #[serde(default)]
    pub partial: bool,
    pub total_checked: usize,
    pub live_count: usize,
    pub recovered_count: usize,
    pub protected_count: usize,
    pub temp_failure_count: usize,
    pub probable_dead_count: usize,
    pub confirmed_dead_count: usize,
    pub needs_review_count: usize,
    pub duration_secs: f64,
    pub confirmed_dead_urls: Vec<String>,
    #[serde(default)]
    pub extreme_confirmed_dead_urls: Vec<String>,
}

pub struct ReportExporter;

impl ReportExporter {
    fn is_strong_structured_content_missing(observation: &BackendObservation) -> bool {
        if observation.error_kind.as_deref() != Some("ContentMissing")
            || !observation
                .status_code
                .is_some_and(|status| (200..300).contains(&status))
            || !observation
                .content_type
                .as_deref()
                .is_some_and(|content_type| content_type.to_ascii_lowercase().contains("html"))
            || !crate::health::content::is_deep_content_url(&observation.initial_url)
            || crate::health::content::is_parked_domain(
                observation.title.as_deref(),
                observation.body_preview.as_deref(),
            )
        {
            return false;
        }

        let visible = format!(
            "{} {}",
            observation.title.as_deref().unwrap_or(""),
            observation.body_preview.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        [
            "video has been removed",
            "video unavailable",
            "video is unavailable",
            "video isn't available",
            "video isn’t available",
            "video does not exist",
        ]
        .iter()
        .any(|marker| visible.contains(marker))
    }

    /// Export reports using the compatibility default of an explicit content scope.
    pub fn export_all<P: AsRef<Path>>(
        out_dir: P,
        run_id: &str,
        duration_secs: f64,
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)], // (bookmark_id, url, verdict, observations)
    ) -> io::Result<HealthRunSummaryReport> {
        Self::export_all_for_scope(
            out_dir,
            run_id,
            duration_secs,
            ProbeScope::Content,
            false,
            results,
        )
    }

    /// Export reports with the probe scope supplied by the caller.
    pub fn export_all_for_scope<P: AsRef<Path>>(
        out_dir: P,
        run_id: &str,
        duration_secs: f64,
        probe_scope: ProbeScope,
        partial: bool,
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)],
    ) -> io::Result<HealthRunSummaryReport> {
        let dir = out_dir.as_ref();
        create_dir_all(dir)?;

        let mut summary = HealthRunSummaryReport {
            run_id: run_id.to_string(),
            probe_scope: scope_name(probe_scope).to_string(),
            partial,
            total_checked: results.len(),
            duration_secs,
            ..Default::default()
        };

        let mut categorized: HashMap<HealthVerdict, Vec<(&str, &str, &[BackendObservation])>> =
            HashMap::new();

        for (bm_id, url, verdict, obs) in results {
            match verdict {
                HealthVerdict::Live => summary.live_count += 1,
                HealthVerdict::Recovered => summary.recovered_count += 1,
                HealthVerdict::Protected => summary.protected_count += 1,
                HealthVerdict::TemporaryFailure => summary.temp_failure_count += 1,
                HealthVerdict::ProbableDead => summary.probable_dead_count += 1,
                HealthVerdict::ConfirmedDead => {
                    summary.confirmed_dead_count += 1;
                    summary.confirmed_dead_urls.push(url.clone());
                    if Self::is_extreme_confirmed_dead(*verdict, obs) {
                        summary.extreme_confirmed_dead_urls.push(url.clone());
                    }
                }
                HealthVerdict::NeedsReview => summary.needs_review_count += 1,
            }
            categorized
                .entry(*verdict)
                .or_default()
                .push((bm_id, url, obs));
        }

        // Export individual category HTMLs
        Self::write_category_html(
            dir.join("live.html"),
            "Live Bookmarks",
            categorized.get(&HealthVerdict::Live).unwrap_or(&vec![]),
        )?;
        Self::write_category_html(
            dir.join("recovered.html"),
            "Recovered Bookmarks",
            categorized
                .get(&HealthVerdict::Recovered)
                .unwrap_or(&vec![]),
        )?;
        Self::write_category_html(
            dir.join("protected.html"),
            "Protected / WAF Bookmarks",
            categorized
                .get(&HealthVerdict::Protected)
                .unwrap_or(&vec![]),
        )?;
        Self::write_category_html(
            dir.join("temporary-failure.html"),
            "Temporary Failure Bookmarks",
            categorized
                .get(&HealthVerdict::TemporaryFailure)
                .unwrap_or(&vec![]),
        )?;
        Self::write_category_html(
            dir.join("probably-dead.html"),
            "Probable Dead Bookmarks",
            categorized
                .get(&HealthVerdict::ProbableDead)
                .unwrap_or(&vec![]),
        )?;
        // Keep the historical misspelling as a compatibility alias.
        Self::write_category_html(
            dir.join("probable-dead.html"),
            "Probable Dead Bookmarks",
            categorized
                .get(&HealthVerdict::ProbableDead)
                .unwrap_or(&vec![]),
        )?;
        Self::write_category_html(
            dir.join("confirmed-dead.html"),
            "Confirmed Dead Bookmarks",
            categorized
                .get(&HealthVerdict::ConfirmedDead)
                .unwrap_or(&vec![]),
        )?;
        Self::write_category_html(
            dir.join("needs-review.html"),
            "Needs Review Bookmarks",
            categorized
                .get(&HealthVerdict::NeedsReview)
                .unwrap_or(&vec![]),
        )?;

        // Export main health-report.html
        Self::write_main_report_html(dir.join("health-report.html"), &summary)?;

        // Export health-summary.json
        let json_data = serde_json::to_string_pretty(&summary)?;
        write_atomic(dir.join("health-summary.json"), json_data.as_bytes())?;
        write_atomic(
            dir.join(format!("{}-health-summary.json", summary.probe_scope)),
            json_data.as_bytes(),
        )?;
        // Preserve the previous scope JSON filename for consumers that already use it.
        write_atomic(
            dir.join(format!("{}-health.json", summary.probe_scope)),
            json_data.as_bytes(),
        )?;

        let scope_summary = format!("{}-health-summary.json", summary.probe_scope);
        let legacy_scope_summary = format!("{}-health.json", summary.probe_scope);
        let scope_observations = format!("{}-health-observations.csv", summary.probe_scope);
        let scope_report = format!("{}-health-report.html", summary.probe_scope);
        Self::write_main_report_html(dir.join(&scope_report), &summary)?;
        Self::write_csv_observations(dir.join(&scope_observations), results)?;

        let manifest = serde_json::json!({
            "schema_version": "health-run-manifest-v1",
            "run_id": run_id,
            "app_version": env!("CARGO_PKG_VERSION"),
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "probe_scope": summary.probe_scope.clone(),
            "total_checked": summary.total_checked,
            // These values describe this report export, not a deletion operation.
            "partial": summary.partial,
            "deleted_count": 0,
            "quarantine_count": 0,
            "artifacts": [
                "health-summary.json",
                scope_summary,
                legacy_scope_summary,
                "health-report.html",
                scope_report,
                "health-observations.csv",
                scope_observations,
                "probably-dead.html",
                "probable-dead.html"
            ]
        });
        write_atomic(
            dir.join("health-run-manifest.json"),
            serde_json::to_string_pretty(&manifest)?.as_bytes(),
        )?;

        // Export health-observations.csv
        Self::write_csv_observations(dir.join("health-observations.csv"), results)?;

        Ok(summary)
    }

    fn is_extreme_confirmed_dead(
        verdict: HealthVerdict,
        observations: &[BackendObservation],
    ) -> bool {
        if verdict != HealthVerdict::ConfirmedDead {
            return false;
        }

        let mut nxdomain_resolvers = HashSet::new();
        let mut http_dead_backends = HashSet::new();
        let mut content_backends_by_hash: HashMap<&str, HashSet<String>> = HashMap::new();

        for observation in observations
            .iter()
            .filter(|observation| observation.actual_request == "yes" && !observation.cache_hit)
        {
            if observation
                .dns_rcode
                .as_deref()
                .is_some_and(|rcode| rcode.eq_ignore_ascii_case("NXDOMAIN"))
            {
                if let Some(resolver_id) = observation
                    .resolver_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|resolver_id| !resolver_id.is_empty())
                {
                    nxdomain_resolvers.insert(resolver_id.to_ascii_lowercase());
                }
            }

            if matches!(observation.status_code, Some(404 | 410)) {
                let backend = observation.backend.trim();
                if !backend.is_empty() {
                    http_dead_backends.insert(backend.to_ascii_lowercase());
                }
            }

            if Self::is_strong_structured_content_missing(observation)
                && observation.content_length.is_some_and(|length| length > 0)
            {
                if let Some(body_hash) = observation
                    .body_hash
                    .as_deref()
                    .map(str::trim)
                    .filter(|body_hash| !body_hash.is_empty())
                {
                    let backend = observation.backend.trim();
                    if !backend.is_empty() {
                        content_backends_by_hash
                            .entry(body_hash)
                            .or_default()
                            .insert(backend.to_ascii_lowercase());
                    }
                }
            }
        }

        nxdomain_resolvers.len() >= 2
            || http_dead_backends.len() >= 2
            || content_backends_by_hash
                .values()
                .any(|backends| backends.len() >= 2)
    }

    fn write_category_html(
        path: impl AsRef<Path>,
        title: &str,
        items: &[(&str, &str, &[BackendObservation])],
    ) -> io::Result<()> {
        let mut output = Vec::new();
        writeln!(output, "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{font-family:sans-serif;margin:2em}} table{{border-collapse:collapse;width:100%}} th,td{{border:1px solid #ccc;padding:8px}} th{{background:#f4f4f4}}</style></head><body>")?;
        writeln!(output, "<h1>{title} (Total: {})</h1>", items.len())?;
        writeln!(
            output,
            "<table><tr><th>Bookmark ID</th><th>URL</th><th>Observations</th></tr>"
        )?;

        for (bm_id, url, obs) in items {
            let obs_summary = obs
                .iter()
                .map(|o| {
                    format!(
                        "{}: status={:?}, err={:?}",
                        o.backend, o.status_code, o.error_kind
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");

            writeln!(
                output,
                "<tr><td>{}</td><td><a href=\"{}\" target=\"_blank\">{}</a></td><td>{}</td></tr>",
                html_escape(bm_id),
                html_escape(url),
                html_escape(url),
                html_escape(&obs_summary)
            )?;
        }

        writeln!(output, "</table></body></html>")?;
        write_atomic(path, &output)
    }

    fn write_main_report_html(
        path: impl AsRef<Path>,
        summary: &HealthRunSummaryReport,
    ) -> io::Result<()> {
        let mut output = Vec::new();
        writeln!(output, "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Bookmark Health Report</title><style>body{{font-family:sans-serif;margin:2em}} ul{{line-height:1.8}} .dead{{color:red;font-weight:bold}} .live{{color:green;font-weight:bold}}</style></head><body>")?;
        writeln!(
            output,
            "<h1>🔍 Bookmark Health Comprehensive Audit Report</h1>"
        )?;
        writeln!(output, "<p>Run ID: <code>{}</code></p>", summary.run_id)?;
        writeln!(
            output,
            "<p>Total URLs Checked: <strong>{}</strong> (Duration: {:.2}s)</p>",
            summary.total_checked, summary.duration_secs
        )?;
        writeln!(output, "<ul>")?;
        writeln!(
            output,
            "<li class=\"live\">Live: {}</li>",
            summary.live_count
        )?;
        writeln!(output, "<li>Recovered: {}</li>", summary.recovered_count)?;
        writeln!(
            output,
            "<li>Protected / WAF: {}</li>",
            summary.protected_count
        )?;
        writeln!(
            output,
            "<li>Temporary Failures: {}</li>",
            summary.temp_failure_count
        )?;
        writeln!(
            output,
            "<li>Probable Dead: {}</li>",
            summary.probable_dead_count
        )?;
        writeln!(
            output,
            "<li class=\"dead\">Confirmed Dead: {}</li>",
            summary.confirmed_dead_count
        )?;
        writeln!(
            output,
            "<li>Needs Review: {}</li>",
            summary.needs_review_count
        )?;
        writeln!(output, "</ul>")?;
        writeln!(output, "<h2>Quarantine & Safety Guidelines</h2>")?;
        writeln!(output, "<p>⚠️ Single-run check results CANNOT be used for destructive deletion. Only <code>ConfirmedDead</code> items verified by 3 runs in the latest 24 hours, with independent verifier families or dual-resolver NXDOMAIN agreement, are eligible for quarantine review.</p>")?;
        writeln!(output, "</body></html>")?;
        write_atomic(path, &output)
    }

    fn write_csv_observations(
        path: impl AsRef<Path>,
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)],
    ) -> io::Result<()> {
        let mut output = Vec::new();
        writeln!(
            output,
            "bookmark_id,url,verdict,backend,status_code,error_kind,response_time_ms"
        )?;

        for (bm_id, url, verdict, observations) in results {
            for obs in observations {
                writeln!(
                    output,
                    "{},{},{},{},{},{},{}",
                    csv_field(bm_id),
                    csv_field(url),
                    csv_field(verdict.as_str()),
                    csv_field(&obs.backend),
                    csv_field(
                        &obs.status_code
                            .map(|code| code.to_string())
                            .unwrap_or_default()
                    ),
                    csv_field(obs.error_kind.as_deref().unwrap_or("")),
                    csv_field(&obs.response_time_ms.to_string())
                )?;
            }
        }
        write_atomic(path, &output)
    }

    /// Only multi-run `ConfirmedDead` verdicts are eligible for deletion.
    pub fn verify_delete_safety(
        verdict: HealthVerdict,
        multi_run_confirmed: bool,
    ) -> Result<(), String> {
        if verdict == HealthVerdict::ConfirmedDead && multi_run_confirmed {
            Ok(())
        } else {
            Err("Deletion requires a multi-run ConfirmedDead verdict.".to_string())
        }
    }

    /// Enforce deletion safety check with explicit 404/NXDOMAIN support:
    /// any single run deletion must be rejected even with explicit 404/410.
    pub fn verify_delete_safety_explicit(
        verdict: HealthVerdict,
        _explicit_unreachable: bool,
        multi_run_confirmed: bool,
    ) -> Result<(), String> {
        Self::verify_delete_safety(verdict, multi_run_confirmed)
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn scope_name(scope: ProbeScope) -> &'static str {
    match scope {
        ProbeScope::Site => "site",
        ProbeScope::Content => "content",
    }
}

fn write_atomic(path: impl AsRef<Path>, data: &[u8]) -> io::Result<()> {
    crate::browser_utils::atomic_write_file(path.as_ref(), data)
        .map_err(|error| io::Error::other(error.to_string()))
}

fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delete_safety_refusal() {
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ProbableDead, false).is_err());
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, false).is_err());
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_ok());

        // Single run explicit unreachable ConfirmedDead must still be rejected
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ConfirmedDead,
            true,
            false
        )
        .is_err());
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ConfirmedDead,
            true,
            true
        )
        .is_ok());
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ProbableDead,
            true,
            false
        )
        .is_err());
    }

    #[test]
    fn test_confirmed_dead_multi_run_can_be_deleted() {
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_ok());
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ConfirmedDead,
            true,
            true
        )
        .is_ok());
    }

    #[test]
    fn export_all_replaces_reports_atomically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("health-summary.json"), "stale").unwrap();

        let report = ReportExporter::export_all(dir.path(), "run-1", 0.5, &[]).unwrap();

        assert_eq!(report.total_checked, 0);
        assert_ne!(
            std::fs::read_to_string(dir.path().join("health-summary.json")).unwrap(),
            "stale"
        );
        assert!(dir.path().join("health-report.html").exists());
        assert!(dir.path().join("health-observations.csv").exists());
        assert!(dir.path().join("content-health.json").exists());
        assert!(dir.path().join("content-health-summary.json").exists());
        assert!(dir.path().join("content-health-observations.csv").exists());
        assert!(dir.path().join("content-health-report.html").exists());
        assert!(dir.path().join("probable-dead.html").exists());
        assert!(dir.path().join("health-run-manifest.json").exists());

        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("health-run-manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["probe_scope"], "content");
        assert_eq!(manifest["total_checked"], 0);
        assert_eq!(manifest["partial"], false);
        assert_eq!(manifest["deleted_count"], 0);
        assert_eq!(manifest["quarantine_count"], 0);
    }

    #[test]
    fn explicit_site_scope_generates_site_reports_for_empty_results() {
        let dir = tempfile::tempdir().unwrap();
        let report = ReportExporter::export_all_for_scope(
            dir.path(),
            "site-run",
            0.1,
            ProbeScope::Site,
            true,
            &[],
        )
        .unwrap();

        assert_eq!(report.probe_scope, "site");
        assert!(dir.path().join("site-health-summary.json").exists());
        assert!(dir.path().join("site-health-observations.csv").exists());
        assert!(dir.path().join("site-health-report.html").exists());
        let summary: HealthRunSummaryReport = serde_json::from_slice(
            &std::fs::read(dir.path().join("site-health-summary.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(summary.probe_scope, "site");
        assert!(summary.partial);
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("health-run-manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["partial"], true);
    }

    #[test]
    fn csv_observations_escape_every_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("health-observations.csv");
        let observation = BackendObservation {
            backend: "native,\"quoted\"\nbackend".to_string(),
            status_code: Some(503),
            error_kind: Some("Server,error \"quoted\"\nnext".to_string()),
            response_time_ms: 42,
            ..Default::default()
        };
        let results = vec![(
            "bookmark,\"id\"\nnext".to_string(),
            "https://example.invalid/?q=\"x\",y\nnext".to_string(),
            HealthVerdict::NeedsReview,
            vec![observation],
        )];

        ReportExporter::write_csv_observations(&path, &results).unwrap();

        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            concat!(
                "bookmark_id,url,verdict,backend,status_code,error_kind,response_time_ms\n",
                "\"bookmark,\"\"id\"\"\nnext\",",
                "\"https://example.invalid/?q=\"\"x\"\",y\nnext\",",
                "\"needs_review\",",
                "\"native,\"\"quoted\"\"\nbackend\",",
                "\"503\",",
                "\"Server,error \"\"quoted\"\"\nnext\",",
                "\"42\"\n"
            )
        );
    }

    #[test]
    fn extreme_confirmation_requires_independent_current_observations() {
        let observation = |backend: &str| BackendObservation {
            backend: backend.to_string(),
            actual_request: "yes".to_string(),
            cache_hit: false,
            ..Default::default()
        };

        let mut cloudflare = observation("native-dns-cloudflare");
        cloudflare.dns_rcode = Some("NXDOMAIN".to_string());
        cloudflare.resolver_id = Some("cloudflare".to_string());
        let mut alidns = observation("native-dns-alidns");
        alidns.dns_rcode = Some("nxdomain".to_string());
        alidns.resolver_id = Some("alidns".to_string());
        assert!(ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &[cloudflare, alidns]
        ));

        let mut native_404 = observation("native-reqwest");
        native_404.status_code = Some(404);
        let mut curl_410 = observation("curl");
        curl_410.status_code = Some(410);
        let http_dead = vec![native_404.clone(), curl_410];
        assert!(ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &http_dead
        ));
        assert!(!ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::Live,
            &http_dead
        ));
        assert!(!ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &[native_404.clone(), native_404]
        ));

        let mut native_missing = observation("native-reqwest");
        native_missing.error_kind = Some("ContentMissing".to_string());
        native_missing.status_code = Some(200);
        native_missing.content_type = Some("text/html".to_string());
        native_missing.initial_url = "https://video.example/watch?v=gone".to_string();
        native_missing.title = Some("Video unavailable".to_string());
        native_missing.content_length = Some(128);
        native_missing.body_hash = Some("same-body".to_string());
        let mut curl_missing = observation("curl");
        curl_missing.error_kind = Some("ContentMissing".to_string());
        curl_missing.status_code = Some(200);
        curl_missing.content_type = Some("text/html".to_string());
        curl_missing.initial_url = "https://video.example/watch?v=gone".to_string();
        curl_missing.title = Some("Video unavailable".to_string());
        curl_missing.content_length = Some(128);
        curl_missing.body_hash = Some("same-body".to_string());
        assert!(ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &[native_missing.clone(), curl_missing.clone()]
        ));
        curl_missing.body_hash = Some("different-body".to_string());
        assert!(!ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &[native_missing, curl_missing]
        ));

        let mut native_parked = observation("native-reqwest");
        native_parked.error_kind = Some("ContentMissing".to_string());
        native_parked.status_code = Some(200);
        native_parked.content_type = Some("text/html".to_string());
        native_parked.initial_url = "https://example.com/old-page".to_string();
        native_parked.title = Some("Buy this domain".to_string());
        native_parked.content_length = Some(128);
        native_parked.body_hash = Some("parked-body".to_string());
        let mut curl_parked = native_parked.clone();
        curl_parked.backend = "curl".to_string();
        assert!(!ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &[native_parked, curl_parked]
        ));

        let mut cached_missing = observation("curl");
        cached_missing.error_kind = Some("ContentMissing".to_string());
        cached_missing.content_length = Some(128);
        cached_missing.body_hash = Some("same-body".to_string());
        cached_missing.cache_hit = true;
        let mut real_missing = observation("native-reqwest");
        real_missing.error_kind = Some("ContentMissing".to_string());
        real_missing.content_length = Some(128);
        real_missing.body_hash = Some("same-body".to_string());
        assert!(!ReportExporter::is_extreme_confirmed_dead(
            HealthVerdict::ConfirmedDead,
            &[real_missing, cached_missing]
        ));

        let dir = tempfile::tempdir().unwrap();
        let weak = vec![http_dead[0].clone()];
        let report = ReportExporter::export_all(
            dir.path(),
            "run-extreme",
            0.1,
            &[
                (
                    "extreme".to_string(),
                    "https://extreme.invalid".to_string(),
                    HealthVerdict::ConfirmedDead,
                    http_dead,
                ),
                (
                    "weak".to_string(),
                    "https://weak.invalid".to_string(),
                    HealthVerdict::ConfirmedDead,
                    weak,
                ),
            ],
        )
        .unwrap();
        assert_eq!(report.confirmed_dead_urls.len(), 2);
        assert_eq!(
            report.extreme_confirmed_dead_urls,
            vec!["https://extreme.invalid"]
        );
    }
}
