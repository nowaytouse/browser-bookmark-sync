//! Source-level import evidence for the native, copy-only bookmark cleaner.
//!
//! Discovery, a successful empty read, and an unsupported collection are different
//! states. A report count is the number of source records before deduplication;
//! it is never evidence of universal browser or Profile coverage.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

use crate::health::review::ReviewDocument;
use crate::sync::{reading_list, ExportConfig, SyncEngine};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceState {
    Imported,
    Empty,
    Detected,
    Unsupported,
    Unavailable,
    PermissionDenied,
    Failed,
    Unverified,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRow {
    pub browser: String,
    pub profile: String,
    /// Stable collection key; the UI supplies its localized label.
    pub collection: String,
    pub state: SourceState,
    /// Successfully read source records, not distinct URLs or retained records.
    pub count: Option<usize>,
    pub detail: String,
}

pub struct BrowserImport {
    pub document: ReviewDocument,
    /// Retain this while the document is open; no browser source is writable.
    pub directory: tempfile::TempDir,
    pub sources: Vec<SourceRow>,
}

/// Read browser/Profile snapshots and reading lists into an isolated HTML copy.
/// Called on the GUI's worker thread, not its event loop. Unread sources remain
/// explicit issues beside successful reads, never silently empty collections.
pub fn collect_browser_copy() -> Result<BrowserImport> {
    let directory = tempfile::Builder::new()
        .prefix("bookmark-cleaner-import-")
        .tempdir()
        .context("create private browser-import directory")?;
    let html = directory.path().join("import.html");
    let report_path = directory.path().join("import-report.json");
    let html_path = html.to_str().context("import path must be UTF-8")?;
    let json_path = report_path
        .to_str()
        .context("source-report path must be UTF-8")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create browser-import worker runtime")?;
    runtime.block_on(async {
        let (extra, reports) = reading_list::collect_for_import("all")
            .context("read supported browser/Profile reading lists")?;
        SyncEngine::new()?
            .export_to_html_with_report(
                None,
                html_path,
                Some(json_path),
                &ExportConfig {
                    no_wrap: true,
                    // Import is lossless. Health checking decides URL eligibility;
                    // unsupported/nonpublic schemes are not silently discarded.
                    exclude_onion: false,
                    ..ExportConfig::default()
                },
                extra,
                reports,
            )
            .await
            .context("export isolated browser/Profile bookmark copies")
    })?;
    let document = ReviewDocument::load(&html, None, Some(&report_path))?;
    let sources = document
        .coverage
        .as_ref()
        .map(rows_from_report)
        .unwrap_or_default();
    validate_import_read_results(&document, &sources)?;
    Ok(BrowserImport {
        document,
        directory,
        sources,
    })
}

fn validate_import_read_results(document: &ReviewDocument, sources: &[SourceRow]) -> Result<()> {
    if document.items.is_empty()
        && sources
            .iter()
            .any(|row| !matches!(row.state, SourceState::Imported | SourceState::Empty))
    {
        let details = sources
            .iter()
            .filter(|row| !matches!(row.state, SourceState::Imported | SourceState::Empty))
            .map(|row| format!("{} / {}: {}", row.browser, row.profile, row.detail))
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("No bookmark links were imported and source coverage is incomplete. Existing content must not be replaced.\n{details}");
    }
    Ok(())
}

/// Accept either the detailed export report or a source-review summary. The
/// summary cannot substantiate individual browser/Profile counts, so it remains
/// an explicitly unverified coverage row instead of invented imported sources.
pub fn rows_from_report(report: &Value) -> Vec<SourceRow> {
    let mut rows = Vec::new();
    let sources = report.get("sources").unwrap_or(report);
    if let Some(installations) = sources.get("installed_browsers").and_then(Value::as_array) {
        for installation in installations {
            let browser = text(installation, "browser_name", "Unknown browser");
            let discovery_error = string(installation, "discovery_error");
            let profiles = installation.get("profiles").and_then(Value::as_array);
            if let Some(error) = discovery_error {
                rows.push(SourceRow {
                    browser: browser.clone(),
                    profile: String::new(),
                    collection: "bookmarks".into(),
                    state: state_from_error(error),
                    count: None,
                    detail: format!("Profile discovery failed: {error}"),
                });
            }
            if profiles.is_none_or(Vec::is_empty) && discovery_error.is_none() {
                rows.push(SourceRow {
                    browser: browser.clone(),
                    profile: String::new(),
                    collection: "bookmarks".into(),
                    state: SourceState::Detected,
                    count: None,
                    detail: "Browser storage was detected, but no bookmark Profile was read. This is not a verified empty collection.".into(),
                });
            }
            for profile in profiles.into_iter().flatten() {
                let count = number(profile, "bookmark_count");
                let error = string(profile, "error");
                let (state, count) =
                    match (profile.get("read_success").and_then(Value::as_bool), error) {
                        (_, Some(error)) => (state_from_error(error), None),
                        (Some(true), None) => successful_read(count),
                        (Some(false), None) => (SourceState::Failed, None),
                        (None, None) => (SourceState::Detected, None),
                    };
                let mut detail = source_detail(profile, "source_path");
                if let Some(error) = error {
                    append_detail(&mut detail, error);
                }
                if state == SourceState::Detected {
                    append_detail(&mut detail, "The report has no successful-read evidence.");
                }
                if browser.contains("Safari") {
                    append_detail(&mut detail, "This count describes the bookmark store; custom Favorites-to-Profile mappings are not separately verified.");
                }
                rows.push(SourceRow {
                    browser: browser.clone(),
                    profile: profile_name(profile),
                    collection: "bookmarks".into(),
                    state,
                    count,
                    detail,
                });
            }
        }
    }
    if let Some(external) = sources
        .get("external_files_and_directories")
        .and_then(Value::as_array)
    {
        for source in external {
            let source_type = string(source, "source_type").unwrap_or("unknown");
            let historical = source_type.to_ascii_lowercase().contains("backup");
            let reading = source_type == "reading_list";
            let discovery = source_type == "browser_discovery";
            let collection = if discovery {
                "browser_discovery"
            } else if reading {
                "reading_list"
            } else if historical {
                "historical_backup"
            } else {
                "external_bookmarks"
            };
            // Source evidence describes the read, not what a later URL filter kept.
            let count = number(source, "bookmark_count");
            let error = string(source, "error");
            let (state, count) = match error {
                Some(error) => (state_from_error(error), None),
                None => external_state(source, count),
            };
            let mut detail = source_detail(
                source,
                if reading || discovery {
                    "path"
                } else {
                    "source_path"
                },
            );
            if let Some(error) = error {
                append_detail(&mut detail, error);
            }
            if let Some(errors) = source.get("read_errors").and_then(Value::as_array) {
                for error in errors {
                    append_detail(&mut detail, &error.to_string());
                }
            }
            if discovery {
                append_detail(&mut detail, "Discovery could not inspect this candidate. It is not a verified browser/Profile or a successfully read empty collection.");
            } else if historical {
                append_detail(&mut detail, "Historical backup records; this does not prove the current browser's bookmarks are covered.");
            } else if source
                .get("detected_jsonlz4_backup_paths")
                .and_then(Value::as_array)
                .is_some_and(|paths| !paths.is_empty())
            {
                append_detail(&mut detail, "Includes historical JSONLZ4 backup records; their counts are not separately attributed to the current browser store.");
            }
            rows.push(SourceRow {
                browser: text(source, "browser", "External source"),
                profile: external_profile_name(source, sources),
                collection: collection.into(),
                state,
                count,
                detail,
            });
        }
    }
    if rows.is_empty() || report.get("limitations").is_some() {
        let limitations = report
            .get("limitations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let mut detail = "This summary does not contain individually verified browser/Profile/collection reads. Load the detailed source report for source-level evidence.".to_owned();
        if !limitations.is_empty() {
            append_detail(&mut detail, &limitations);
        }
        rows.push(SourceRow {
            browser: "Source coverage".into(),
            profile: String::new(),
            collection: "source_coverage".into(),
            state: SourceState::Unverified,
            count: None,
            detail,
        });
    }
    rows.sort_by(|a, b| {
        (&a.browser, &a.profile, &a.collection).cmp(&(&b.browser, &b.profile, &b.collection))
    });
    rows
}

fn external_state(source: &Value, count: Option<usize>) -> (SourceState, Option<usize>) {
    match string(source, "status") {
        Some("complete" | "ok" | "imported" | "success") => {
            if source
                .get("read_errors")
                .and_then(Value::as_array)
                .is_some_and(|errors| !errors.is_empty())
            {
                (SourceState::Unverified, count.filter(|count| *count > 0))
            } else {
                successful_read(count)
            }
        }
        Some("unsupported") => (SourceState::Unsupported, None),
        Some("not_present" | "unavailable" | "missing") => (SourceState::Unavailable, None),
        Some("permission_denied") => (SourceState::PermissionDenied, None),
        Some("failed" | "error") => (SourceState::Failed, None),
        Some("detected") => (SourceState::Detected, None),
        Some("skipped") => (SourceState::Skipped, None),
        Some("partial") => (SourceState::Unverified, count.filter(|count| *count > 0)),
        // Existing direct-file import reports have no status field, but their
        // producer only emits these known shapes after a successful read.
        None if matches!(
            string(source, "source_type"),
            Some("Firefox/Waterfox JSONLZ4 Backup" | "SQLite Database" | "HTML Bookmark File")
        ) =>
        {
            successful_read(count)
        }
        _ => (SourceState::Unverified, None),
    }
}

fn successful_read(count: Option<usize>) -> (SourceState, Option<usize>) {
    match count {
        Some(0) => (SourceState::Empty, Some(0)),
        Some(count) => (SourceState::Imported, Some(count)),
        None => (SourceState::Unverified, None),
    }
}

fn state_from_error(error: &str) -> SourceState {
    let lower = error.to_ascii_lowercase();
    if lower.contains("permission denied")
        || lower.contains("operation not permitted")
        || lower.contains("access is denied")
    {
        SourceState::PermissionDenied
    } else if lower.contains("no such file")
        || lower.contains("file not found")
        || lower.contains("cannot find the file")
    {
        SourceState::Unavailable
    } else {
        SourceState::Failed
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn text(value: &Value, key: &str, fallback: &str) -> String {
    string(value, key).unwrap_or(fallback).to_owned()
}

fn number(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
}

fn source_detail(value: &Value, key: &str) -> String {
    text(value, key, "No source path was recorded.")
}

fn append_detail(detail: &mut String, value: &str) {
    detail.push('\n');
    detail.push_str(value);
}

fn profile_name(profile: &Value) -> String {
    let name = string(profile, "profile_name");
    let id = string(profile, "profile_id");
    match (name, id) {
        (Some(name), Some(id)) if name != id => format!("{name} ({id})"),
        (Some(name), _) => name.into(),
        (_, Some(id)) => id.into(),
        _ => "Unidentified Profile".into(),
    }
}

fn external_profile_name(source: &Value, sources: &Value) -> String {
    let browser = string(source, "browser");
    let path = string(source, "path").map(Path::new);
    if let (Some(browser), Some(path)) = (browser, path) {
        if let Some(installations) = sources.get("installed_browsers").and_then(Value::as_array) {
            for installation in installations {
                if string(installation, "browser_name") != Some(browser) {
                    continue;
                }
                for profile in installation
                    .get("profiles")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if string(profile, "source_path").is_some_and(|source| path.starts_with(source))
                    {
                        return profile_name(profile);
                    }
                }
            }
        }
        // Chromium reading-only Profiles can be absent from the bookmarks list.
        // The collector records their canonical store suffix and human name.
        if path.ends_with(Path::new("Sync Data/LevelDB")) {
            if let Some(id) = path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|id| id.to_str())
            {
                let name = string(source, "profile").unwrap_or(id);
                return if name == id {
                    id.into()
                } else {
                    format!("{name} ({id})")
                };
            }
        }
    }
    text(source, "profile", "Unidentified Profile")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unverified_discovery_is_not_a_browser_or_empty_collection() {
        let rows = rows_from_report(&json!({"sources":{"external_files_and_directories":[{
            "source_type":"browser_discovery", "browser":"Unclassified directory", "profile":"discord",
            "path":"/test/discord", "status":"unverified", "browser_identity_verified":false,
            "error":"Operation not permitted (os error 1)"
        }]}}));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].collection, "browser_discovery");
        assert_eq!(rows[0].state, SourceState::PermissionDenied);
        assert_eq!(rows[0].count, None);
        assert!(rows[0].detail.contains("not a verified browser/Profile"));
    }

    #[test]
    fn empty_partial_import_is_rejected_instead_of_replacing_previous_content() -> Result<()> {
        let input =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/review-bookmarks.html");
        let mut document = ReviewDocument::load(&input, None, None)?;
        let denied = SourceRow {
            browser: "Safari".into(),
            profile: "Default".into(),
            collection: "bookmarks".into(),
            state: SourceState::PermissionDenied,
            count: None,
            detail: "Operation not permitted".into(),
        };
        validate_import_read_results(&document, std::slice::from_ref(&denied))?;
        document.items.clear();
        assert!(validate_import_read_results(&document, &[denied]).is_err());
        Ok(())
    }

    #[test]
    fn failed_missing_and_unsupported_are_not_empty() {
        let report = json!({"sources":{"installed_browsers":[{
        "browser_name":"Browser", "profiles":[
            {"profile_name":"Empty", "read_success":true, "bookmark_count":0},
            {"profile_name":"Denied", "read_success":false, "bookmark_count":0, "error":"Permission denied (os error 13)"},
            {"profile_name":"Failed", "read_success":false, "bookmark_count":0},
            {"profile_name":"Detected", "bookmark_count":0},
            {"profile_name":"No count", "read_success":true}
        ]}], "external_files_and_directories":[
            {"browser":"Browser", "profile":"Unsupported", "source_type":"reading_list", "status":"unsupported", "bookmark_count":0},
            {"browser":"Browser", "profile":"Missing", "source_type":"reading_list", "status":"not_present", "bookmark_count":0}
        ]}});
        let rows = rows_from_report(&report);
        let row = |name: &str| rows.iter().find(|row| row.profile == name).unwrap();
        assert_eq!(
            (row("Empty").state, row("Empty").count),
            (SourceState::Empty, Some(0))
        );
        for (name, state) in [
            ("Denied", SourceState::PermissionDenied),
            ("Failed", SourceState::Failed),
            ("Detected", SourceState::Detected),
            ("No count", SourceState::Unverified),
            ("Unsupported", SourceState::Unsupported),
            ("Missing", SourceState::Unavailable),
        ] {
            assert_eq!((row(name).state, row(name).count), (state, None));
        }
    }

    #[test]
    fn reading_list_joins_profile_identity_without_merging_collections() {
        let report = json!({"sources":{"installed_browsers":[{
            "browser_name":"Chrome", "profiles":[{"profile_id":"Profile 1", "profile_name":"Work", "source_path":"/test/Profile 1", "read_success":true, "bookmark_count":8}]
        }], "external_files_and_directories":[
            {"browser":"Chrome", "profile":"Work", "path":"/test/Profile 1/Sync Data/LevelDB", "source_type":"reading_list", "status":"complete", "bookmark_count":2},
            {"browser":"Chrome", "profile":"Read only", "path":"/test/Profile 2/Sync Data/LevelDB", "source_type":"reading_list", "status":"complete", "bookmark_count":0}
        ]}});
        let rows = rows_from_report(&report);
        assert_eq!(rows.len(), 3);
        let work = rows
            .iter()
            .filter(|row| row.profile == "Work (Profile 1)")
            .collect::<Vec<_>>();
        assert_eq!(work.len(), 2);
        assert_ne!(work[0].collection, work[1].collection);
        assert!(rows
            .iter()
            .any(|row| row.profile == "Read only (Profile 2)" && row.state == SourceState::Empty));
    }

    #[test]
    fn installation_without_profiles_is_detected_not_zero() {
        let rows = rows_from_report(
            &json!({"sources":{"installed_browsers":[{"browser_name":"Arc", "profiles":[], "total_bookmarks_collected":0}]}}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].state, rows[0].count),
            (SourceState::Detected, None)
        );
    }

    #[test]
    fn summary_does_not_invent_individual_imports() {
        let rows = rows_from_report(
            &json!({"schema_version":"source-review-v1", "bookmark_profiles":22, "universal_source_coverage_verified":false, "limitations":["Safari Favorites mapping remains unverified."]}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, SourceState::Unverified);
        assert_eq!(rows[0].count, None);
        assert!(rows[0].detail.contains("Favorites mapping"));
    }

    #[test]
    fn historical_backup_and_partial_directory_are_explicit() {
        let rows = rows_from_report(&json!({"sources":{"external_files_and_directories":[
            {"source_type":"Firefox/Waterfox JSONLZ4 Backup", "source_path":"/old/bookmarks.jsonlz4", "bookmark_count":12},
            {"source_type":"Firefox/Waterfox Profile Directory", "source_path":"/old", "status":"partial", "bookmark_count":0, "read_errors":[{"error":"bad backup"}]}
        ]}}));
        let backup = rows
            .iter()
            .find(|row| row.collection == "historical_backup")
            .unwrap();
        assert_eq!(
            (backup.state, backup.count),
            (SourceState::Imported, Some(12))
        );
        assert!(backup.detail.contains("Historical backup"));
        let partial = rows
            .iter()
            .find(|row| row.collection == "external_bookmarks")
            .unwrap();
        assert_eq!(
            (partial.state, partial.count),
            (SourceState::Unverified, None)
        );
    }

    #[test]
    fn contradictory_success_and_invalid_count_fail_closed() {
        let rows = rows_from_report(&json!({"sources":{"external_files_and_directories":[
            {"browser":"A", "source_type":"reading_list", "status":"complete", "bookmark_count":9, "error":"parse failure"},
            {"browser":"B", "source_type":"reading_list", "status":"complete", "bookmark_count":-1},
            {"browser":"C", "source_type":"reading_list", "status":"complete", "bookmark_count":0, "read_errors":[{"error":"partial parse"}]},
            {"browser":"D", "source_type":"reading_list", "status":"unknown_new_state", "bookmark_count":0}
        ]}}));
        assert_eq!(rows[0].state, SourceState::Failed);
        assert_eq!(rows[1].state, SourceState::Unverified);
        assert_eq!(rows[2].state, SourceState::Unverified);
        assert_eq!(rows[3].state, SourceState::Unverified);
        assert!(rows.iter().all(|row| row.count.is_none()));
    }

    #[test]
    fn filtered_zero_does_not_claim_a_nonempty_source_is_empty() {
        let rows = rows_from_report(&json!({"sources":{"external_files_and_directories":[
            {"browser":"A", "source_type":"reading_list", "status":"complete", "bookmark_count":4, "bookmark_count_after_filters":0}
        ]}}));
        assert_eq!(
            (rows[0].state, rows[0].count),
            (SourceState::Imported, Some(4))
        );
    }
}
