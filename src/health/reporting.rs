//! Health Reports Generation and Delete Safety Enforcement

use crate::health::models::{BackendObservation, HealthVerdict};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::create_dir_all;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthRunSummaryReport {
    pub run_id: String,
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
}

pub struct ReportExporter;

impl ReportExporter {
    /// Export all 8 HTML reports, JSON summary, and CSV observations to target directory
    pub fn export_all<P: AsRef<Path>>(
        out_dir: P,
        run_id: &str,
        duration_secs: f64,
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)], // (bookmark_id, url, verdict, observations)
    ) -> io::Result<HealthRunSummaryReport> {
        let dir = out_dir.as_ref();
        create_dir_all(dir)?;

        let mut summary = HealthRunSummaryReport {
            run_id: run_id.to_string(),
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

        // Export health-observations.csv
        Self::write_csv_observations(dir.join("health-observations.csv"), results)?;

        Ok(summary)
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
                    "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{}",
                    csv_escape(bm_id),
                    csv_escape(url),
                    verdict.as_str(),
                    obs.backend,
                    obs.status_code.map(|c| c.to_string()).unwrap_or_default(),
                    obs.error_kind.as_deref().unwrap_or(""),
                    obs.response_time_ms
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

fn write_atomic(path: impl AsRef<Path>, data: &[u8]) -> io::Result<()> {
    crate::browser_utils::atomic_write_file(path.as_ref(), data)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))
}

fn csv_escape(s: &str) -> String {
    s.replace('"', "\"\"")
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
    }
}
