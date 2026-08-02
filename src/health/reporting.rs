//! Health Reports Generation and Delete Safety Enforcement

use crate::health::models::{BackendObservation, HealthVerdict};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::Write;
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
}

pub struct ReportExporter;

impl ReportExporter {
    /// Export all 8 HTML reports, JSON summary, and CSV observations to target directory
    pub fn export_all<P: AsRef<Path>>(
        out_dir: P,
        run_id: &str,
        duration_secs: f64,
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)], // (bookmark_id, url, verdict, observations)
    ) -> std::io::Result<HealthRunSummaryReport> {
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
                HealthVerdict::ConfirmedDead => summary.confirmed_dead_count += 1,
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
        let mut json_file = File::create(dir.join("health-summary.json"))?;
        json_file.write_all(json_data.as_bytes())?;

        // Export health-observations.csv
        Self::write_csv_observations(dir.join("health-observations.csv"), results)?;

        Ok(summary)
    }

    fn write_category_html(
        path: impl AsRef<Path>,
        title: &str,
        items: &[(&str, &str, &[BackendObservation])],
    ) -> std::io::Result<()> {
        let mut file = File::create(path)?;
        writeln!(file, "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{font-family:sans-serif;margin:2em}} table{{border-collapse:collapse;width:100%}} th,td{{border:1px solid #ccc;padding:8px}} th{{background:#f4f4f4}}</style></head><body>")?;
        writeln!(file, "<h1>{title} (Total: {})</h1>", items.len())?;
        writeln!(
            file,
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
                file,
                "<tr><td>{}</td><td><a href=\"{}\" target=\"_blank\">{}</a></td><td>{}</td></tr>",
                html_escape(bm_id),
                html_escape(url),
                html_escape(url),
                html_escape(&obs_summary)
            )?;
        }

        writeln!(file, "</table></body></html>")?;
        Ok(())
    }

    fn write_main_report_html(
        path: impl AsRef<Path>,
        summary: &HealthRunSummaryReport,
    ) -> std::io::Result<()> {
        let mut file = File::create(path)?;
        writeln!(file, "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Bookmark Health Report</title><style>body{{font-family:sans-serif;margin:2em}} ul{{line-height:1.8}} .dead{{color:red;font-weight:bold}} .live{{color:green;font-weight:bold}}</style></head><body>")?;
        writeln!(
            file,
            "<h1>🔍 Bookmark Health Comprehensive Audit Report</h1>"
        )?;
        writeln!(file, "<p>Run ID: <code>{}</code></p>", summary.run_id)?;
        writeln!(
            file,
            "<p>Total URLs Checked: <strong>{}</strong> (Duration: {:.2}s)</p>",
            summary.total_checked, summary.duration_secs
        )?;
        writeln!(file, "<ul>")?;
        writeln!(file, "<li class=\"live\">Live: {}</li>", summary.live_count)?;
        writeln!(file, "<li>Recovered: {}</li>", summary.recovered_count)?;
        writeln!(
            file,
            "<li>Protected / WAF: {}</li>",
            summary.protected_count
        )?;
        writeln!(
            file,
            "<li>Temporary Failures: {}</li>",
            summary.temp_failure_count
        )?;
        writeln!(
            file,
            "<li>Probable Dead: {}</li>",
            summary.probable_dead_count
        )?;
        writeln!(
            file,
            "<li class=\"dead\">Confirmed Dead: {}</li>",
            summary.confirmed_dead_count
        )?;
        writeln!(
            file,
            "<li>Needs Review: {}</li>",
            summary.needs_review_count
        )?;
        writeln!(file, "</ul>")?;
        writeln!(file, "<h2>Quarantine & Safety Guidelines</h2>")?;
        writeln!(file, "<p>⚠️ Single-run check results CANNOT be used for destructive deletion. Only <code>ConfirmedDead</code> items (verified across 3 runs over 7 days by independent tools) are eligible for quarantine review.</p>")?;
        writeln!(file, "</body></html>")?;
        Ok(())
    }

    fn write_csv_observations(
        path: impl AsRef<Path>,
        results: &[(String, String, HealthVerdict, Vec<BackendObservation>)],
    ) -> std::io::Result<()> {
        let mut file = File::create(path)?;
        writeln!(
            file,
            "bookmark_id,url,verdict,backend,status_code,error_kind,response_time_ms"
        )?;

        for (bm_id, url, verdict, observations) in results {
            for obs in observations {
                writeln!(
                    file,
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
        Ok(())
    }

    /// Enforce deletion safety check: refuse single-run deletion and refuse automated destructive deletion in Phase 1
    pub fn verify_delete_safety(
        _verdict: HealthVerdict,
        _multi_run_confirmed: bool,
    ) -> Result<(), String> {
        Err("Automated destructive deletion is disabled in Phase 1. ConfirmedDead is quarantine-only.".to_string())
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
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_err());

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
        .is_err());
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ProbableDead,
            true,
            false
        )
        .is_err());
    }

    #[test]
    fn test_confirmed_dead_is_quarantine_only() {
        assert!(ReportExporter::verify_delete_safety(HealthVerdict::ConfirmedDead, true).is_err());
        assert!(ReportExporter::verify_delete_safety_explicit(
            HealthVerdict::ConfirmedDead,
            true,
            true
        )
        .is_err());
    }
}
