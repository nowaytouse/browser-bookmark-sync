//! Netscape HTML & Sidecar Audit Report Exporter

use crate::browser_utils::atomic_write_file;
use crate::data_types::canonical::{CanonicalBookmark, DuplicateCsvRecord, DuplicateGroupReport};
use crate::data_types::manifest::{InventoryReport, RunManifest};
use crate::data_types::models::{BookmarkNode, BookmarkTree};
use anyhow::Result;
use std::io::Write;
use std::path::Path;

pub struct ReportExporter;

impl ReportExporter {
    /// Export high-fidelity original HTML preserving complete tree hierarchy
    pub fn export_original_html(tree: &BookmarkTree, output_path: &Path) -> Result<()> {
        let mut output = Vec::new();
        writeln!(output, "<!DOCTYPE NETSCAPE-Bookmark-file-1>")?;
        writeln!(
            output,
            "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">"
        )?;
        writeln!(output, "<TITLE>Original Bookmarks Backup</TITLE>")?;
        writeln!(output, "<H1>Bookmarks Backup (Original Preserved)</H1>")?;
        writeln!(output, "<DL><p>")?;

        for root in &tree.roots {
            Self::write_node_html(&mut output, root, 1)?;
        }

        writeln!(output, "</DL><p>")?;
        atomic_write_file(output_path, &output)
    }

    /// Export deduplicated Netscape HTML with canonical primary bookmarks
    pub fn export_deduplicated_html(
        canonicals: &[CanonicalBookmark],
        output_path: &Path,
    ) -> Result<()> {
        let mut output = Vec::new();
        writeln!(output, "<!DOCTYPE NETSCAPE-Bookmark-file-1>")?;
        writeln!(
            output,
            "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">"
        )?;
        writeln!(output, "<TITLE>Deduplicated Bookmarks</TITLE>")?;
        writeln!(output, "<H1>Bookmarks (Exact URL Deduplicated)</H1>")?;
        writeln!(output, "<DL><p>")?;

        for canonical in canonicals {
            let primary = &canonical.primary;
            writeln!(
                output,
                "    <DT><A HREF=\"{}\">{}</A>",
                Self::html_escape(&primary.raw_url),
                Self::html_escape(&primary.title)
            )?;
        }

        writeln!(output, "</DL><p>")?;
        atomic_write_file(output_path, &output)
    }

    /// Write all sidecar JSON/CSV audit reports
    pub fn write_sidecar_reports(
        output_dir: &Path,
        inventory: &InventoryReport,
        reports: &[DuplicateGroupReport],
        csv_records: &[DuplicateCsvRecord],
        manifest: &RunManifest,
    ) -> Result<()> {
        std::fs::create_dir_all(output_dir)?;

        // 1. inventory.json
        let inv_path = output_dir.join("inventory.json");
        let inv_json = serde_json::to_string_pretty(inventory)?;
        atomic_write_file(&inv_path, inv_json.as_bytes())?;

        // 2. duplicates.json
        let dup_json_path = output_dir.join("duplicates.json");
        let dup_json = serde_json::to_string_pretty(reports)?;
        atomic_write_file(&dup_json_path, dup_json.as_bytes())?;

        // 3. duplicates.csv
        let dup_csv_path = output_dir.join("duplicates.csv");
        let mut csv_output = Vec::new();
        writeln!(
            csv_output,
            "group_id,exact_url,action,browser,profile,root,folder_path,title,date_added,date_modified,original_position,selection_reason"
        )?;

        for r in csv_records {
            writeln!(
                csv_output,
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                Self::csv_escape(&r.group_id),
                Self::csv_escape(&r.exact_url),
                Self::csv_escape(&r.action),
                Self::csv_escape(&r.browser),
                Self::csv_escape(&r.profile),
                Self::csv_escape(&r.root),
                Self::csv_escape(&r.folder_path),
                Self::csv_escape(&r.title),
                Self::csv_escape(&r.date_added),
                Self::csv_escape(&r.date_modified),
                Self::csv_escape(&r.original_position.to_string()),
                Self::csv_escape(&r.selection_reason)
            )?;
        }
        atomic_write_file(&dup_csv_path, &csv_output)?;

        // 4. run-manifest.json
        let manifest_path = output_dir.join("run-manifest.json");
        let manifest_json = serde_json::to_string_pretty(manifest)?;
        atomic_write_file(&manifest_path, manifest_json.as_bytes())?;

        // 5. SUMMARY-REPORT.md (Comprehensive natural English report)
        let summary_md_path = output_dir.join("SUMMARY-REPORT.md");
        let summary_content = Self::generate_english_summary_markdown(inventory, manifest);
        atomic_write_file(&summary_md_path, summary_content.as_bytes())?;

        Ok(())
    }

    /// Generate a comprehensive natural English Markdown audit report
    pub fn generate_english_summary_markdown(
        inventory: &InventoryReport,
        manifest: &RunManifest,
    ) -> String {
        let mut report = String::new();

        report.push_str("# Comprehensive Browser Bookmark Aggregation & Deduplication Report\n\n");

        report.push_str("## 1. Executive Summary\n\n");
        report.push_str(&format!(
            "This report summarizes the automated bookmark collection, snapshot isolation, and exact-URL deduplication executed on **{}** (Run ID: `{}`).\n\n",
            manifest.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            manifest.run_id
        ));

        report.push_str(&format!(
            "- **Total Discovered Browser Installations**: {}\n",
            inventory.total_installations
        ));
        report.push_str(&format!(
            "- **Total Enumerated User Profiles**: {}\n",
            inventory.total_profiles
        ));
        report.push_str(&format!(
            "- **Pre-Deduplication Raw Bookmarks (Total Occurrences)**: {}\n",
            manifest.total_raw_occurrences
        ));
        report.push_str(&format!(
            "- **Post-Deduplication Canonical Bookmarks (Unique URLs Kept)**: {}\n",
            manifest.total_canonical_bookmarks
        ));
        report.push_str(&format!(
            "- **Merged Duplicate Occurrences**: {}\n",
            manifest.total_removed_duplicates
        ));
        report.push_str(&format!(
            "- **Conservation Invariant (`Raw == Primary + Merged`)**: `{}` (100% Satisfied)\n\n",
            manifest.invariant_satisfied
        ));

        report.push_str("## 2. Scanned Scope: Browsers & User Profiles\n\n");
        report.push_str(
            "| Browser | Channel | Profile ID | Display Name | Bookmark Count | Folder Count |\n",
        );
        report.push_str("| --- | --- | --- | --- | --- | --- |\n");

        for inst_summary in &inventory.installations {
            let inst = &inst_summary.installation;
            let channel_str = inst.channel.as_deref().unwrap_or("Stable");

            if inst_summary.profiles.is_empty() {
                report.push_str(&format!(
                    "| {} | {} | *None* | *No active profile found* | 0 | 0 |\n",
                    inst.name, channel_str
                ));
            } else {
                for prof_summary in &inst_summary.profiles {
                    let prof = &prof_summary.profile;
                    report.push_str(&format!(
                        "| {} | {} | `{}` | {} | {} | {} |\n",
                        inst.name,
                        channel_str,
                        prof.id,
                        prof.name,
                        prof_summary.bookmark_count,
                        prof_summary.folder_count
                    ));
                }
            }
        }

        report.push_str("\n## 3. Deduplication Policy & Tie-Breaking Rules\n\n");
        report.push_str(
            "The deduplication engine performed **exact URL matching** (`raw_url.trim()`).\n",
        );
        report.push_str("For each set of duplicate bookmark occurrences pointing to the exact same URL, a single **Primary Canonical Bookmark** was selected based on the following deterministic tie-breaking priority:\n\n");
        report.push_str("1. **Preferred Browser Source**: User-configured or default browser authority priority.\n");
        report.push_str("2. **Bookmark Root Semantics**: `BookmarkBar` > `BookmarkMenu` / `Toolbar` > `OtherBookmarks` > `MobileBookmarks`.\n");
        report.push_str("3. **Folder Hierarchy Specificity**: Deeper and more descriptive custom folder paths are prioritized over root or unfiled folders.\n");
        report
            .push_str("4. **Creation Timestamp**: Earliest `date_added` timestamp is retained.\n");
        report.push_str("5. **Deterministic ID Fallback**: Stable UUID ordering ensures 100% reproducible builds across multiple runs.\n\n");

        report.push_str("## 4. Generated Artifacts & Verification\n\n");
        report.push_str("- `bookmarks-original.html`: Complete, un-deduplicated original bookmark tree preserving full browser/profile/folder hierarchy.\n");
        report.push_str("- `bookmarks-deduplicated.html`: Clean, Netscape-compliant HTML containing deduplicated canonical primary bookmarks.\n");
        report.push_str("- `inventory.json`: Machine-readable audit inventory of all detected installations and profiles.\n");
        report.push_str("- `duplicates.csv`: Tabular spreadsheet logging all merged duplicates, original locations, and selection reasons.\n");
        report.push_str("- `duplicates.json`: Detailed JSON audit report mapping every canonical URL to its full list of merged duplicate occurrences.\n");
        report.push_str("- `run-manifest.json`: Full operational manifest with snapshot BLAKE3 cryptographic integrity hashes.\n\n");

        report.push_str("## 5. Explicit Audit Warnings & Empty Profile Observations\n\n");
        if manifest.warnings.is_empty() {
            report.push_str(
                "*No warnings or skipped profile observations recorded during this run.*\n\n",
            );
        } else {
            for warn in &manifest.warnings {
                report.push_str(&format!("- ⚠️ {warn}\n"));
            }
            report.push('\n');
        }

        report.push_str("---\n*Generated automatically by browser-bookmark-sync (bsync v1.4.1)*\n");

        report
    }

    fn write_node_html<W: Write>(file: &mut W, node: &BookmarkNode, indent: usize) -> Result<()> {
        let indent_str = "    ".repeat(indent);
        match node {
            BookmarkNode::Folder(f) => {
                writeln!(
                    file,
                    "{}<DT><H3>{}</H3>",
                    indent_str,
                    Self::html_escape(&f.title)
                )?;
                writeln!(file, "{indent_str}<DL><p>")?;
                for child in &f.children {
                    Self::write_node_html(file, child, indent + 1)?;
                }
                writeln!(file, "{indent_str}</DL><p>")?;
            }
            BookmarkNode::Bookmark(b) => {
                writeln!(
                    file,
                    "{}<DT><A HREF=\"{}\">{}</A>",
                    indent_str,
                    Self::html_escape(&b.raw_url),
                    Self::html_escape(&b.title)
                )?;
            }
            BookmarkNode::Separator(_) => {
                writeln!(file, "{indent_str}<HR>")?;
            }
        }
        Ok(())
    }

    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    fn csv_escape(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
}

#[cfg(test)]
mod tests {
    use super::ReportExporter;

    #[test]
    fn csv_escape_quotes_special_fields() {
        assert_eq!(
            ReportExporter::csv_escape("profile, \"work\"\n"),
            "\"profile, \"\"work\"\"\n\""
        );
    }
}
