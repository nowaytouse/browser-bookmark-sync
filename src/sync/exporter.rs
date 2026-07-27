//! Netscape HTML & Sidecar Audit Report Exporter

use crate::data_types::canonical::{CanonicalBookmark, DuplicateCsvRecord, DuplicateGroupReport};
use crate::data_types::manifest::{InventoryReport, RunManifest};
use crate::data_types::models::{BookmarkNode, BookmarkTree};
use anyhow::Result;
use std::fs::File;
use std::io::Write;
use std::path::Path;

pub struct ReportExporter;

impl ReportExporter {
    /// Export high-fidelity original HTML preserving complete tree hierarchy
    pub fn export_original_html(tree: &BookmarkTree, output_path: &Path) -> Result<()> {
        let mut file = File::create(output_path)?;
        writeln!(file, "<!DOCTYPE NETSCAPE-Bookmark-file-1>")?;
        writeln!(
            file,
            "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">"
        )?;
        writeln!(file, "<TITLE>Original Bookmarks Backup</TITLE>")?;
        writeln!(file, "<H1>Bookmarks Backup (Original Preserved)</H1>")?;
        writeln!(file, "<DL><p>")?;

        for root in &tree.roots {
            Self::write_node_html(&mut file, root, 1)?;
        }

        writeln!(file, "</DL><p>")?;
        file.flush()?;
        Ok(())
    }

    /// Export deduplicated Netscape HTML with canonical primary bookmarks
    pub fn export_deduplicated_html(
        canonicals: &[CanonicalBookmark],
        output_path: &Path,
    ) -> Result<()> {
        let mut file = File::create(output_path)?;
        writeln!(file, "<!DOCTYPE NETSCAPE-Bookmark-file-1>")?;
        writeln!(
            file,
            "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">"
        )?;
        writeln!(file, "<TITLE>Deduplicated Bookmarks</TITLE>")?;
        writeln!(file, "<H1>Bookmarks (Exact URL Deduplicated)</H1>")?;
        writeln!(file, "<DL><p>")?;

        for canonical in canonicals {
            let primary = &canonical.primary;
            writeln!(
                file,
                "    <DT><A HREF=\"{}\">{}</A>",
                Self::html_escape(&primary.raw_url),
                Self::html_escape(&primary.title)
            )?;
        }

        writeln!(file, "</DL><p>")?;
        file.flush()?;
        Ok(())
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
        std::fs::write(inv_path, inv_json)?;

        // 2. duplicates.json
        let dup_json_path = output_dir.join("duplicates.json");
        let dup_json = serde_json::to_string_pretty(reports)?;
        std::fs::write(dup_json_path, dup_json)?;

        // 3. duplicates.csv
        let dup_csv_path = output_dir.join("duplicates.csv");
        let mut csv_file = File::create(dup_csv_path)?;
        writeln!(
            csv_file,
            "group_id,exact_url,action,browser,profile,root,folder_path,title,date_added,date_modified,original_position,selection_reason"
        )?;

        for r in csv_records {
            writeln!(
                csv_file,
                "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\"",
                r.group_id,
                Self::csv_escape(&r.exact_url),
                r.action,
                r.browser,
                r.profile,
                r.root,
                Self::csv_escape(&r.folder_path),
                Self::csv_escape(&r.title),
                r.date_added,
                r.date_modified,
                r.original_position,
                r.selection_reason
            )?;
        }
        csv_file.flush()?;

        // 4. run-manifest.json
        let manifest_path = output_dir.join("run-manifest.json");
        let manifest_json = serde_json::to_string_pretty(manifest)?;
        std::fs::write(manifest_path, manifest_json)?;

        Ok(())
    }

    fn write_node_html(file: &mut File, node: &BookmarkNode, indent: usize) -> Result<()> {
        let indent_str = "    ".repeat(indent);
        match node {
            BookmarkNode::Folder(f) => {
                writeln!(
                    file,
                    "{}<DT><H3>{}</H3>",
                    indent_str,
                    Self::html_escape(&f.title)
                )?;
                writeln!(file, "{}<DL><p>", indent_str)?;
                for child in &f.children {
                    Self::write_node_html(file, child, indent + 1)?;
                }
                writeln!(file, "{}</DL><p>", indent_str)?;
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
                writeln!(file, "{}<HR>", indent_str)?;
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
        s.replace('"', "\"\"")
    }
}
