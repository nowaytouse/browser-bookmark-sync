//! Explicit, copy-only quarantine selection and application.

use crate::browsers::Bookmark;
use crate::health::db::HealthDatabase;
use crate::url_checker::{
    count_bookmarks, remove_invalid_bookmarks_preserve_structure, RemoveConfig,
};
use anyhow::{bail, ensure, Context, Result};
use blake3::Hasher;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const SELECTION_SCHEMA: &str = "quarantine-selection-v1";
const AUDIT_SCHEMA: &str = "deletion-audit-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuarantineItem {
    pub run_id: String,
    pub bookmark_id: String,
    pub original_url: String,
    pub reason: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineSelection {
    pub schema_version: String,
    pub generated_at: DateTime<Utc>,
    pub items: Vec<QuarantineItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletionAudit {
    pub schema_version: String,
    pub generated_at: DateTime<Utc>,
    pub selection_path: String,
    pub input_path: String,
    pub output_path: String,
    pub input_hash_before: String,
    pub input_hash_after: String,
    pub source_count: usize,
    pub kept_count: usize,
    pub removed_count: usize,
    pub output_count: usize,
    pub selection_count: usize,
    pub removed_items: Vec<QuarantineItem>,
}

pub fn write_selection(db: &HealthDatabase, output: &Path) -> Result<usize> {
    let items = db
        .get_quarantine_candidates_detailed()
        .context("query quarantine candidates")?
        .into_iter()
        .map(|(bookmark_id, original_url, run_id, verdict, checked_at)| {
            let scope = if bookmark_id.starts_with("site:") {
                "site"
            } else {
                "content"
            };
            QuarantineItem {
                run_id,
                bookmark_id,
                original_url,
                reason: format!("{verdict} in completed run within 24h; checked_at={checked_at}"),
                scope: scope.to_string(),
            }
        })
        .collect::<Vec<_>>();

    let selection = QuarantineSelection {
        schema_version: SELECTION_SCHEMA.to_string(),
        generated_at: Utc::now(),
        items,
    };
    let data = serde_json::to_vec_pretty(&selection).context("serialize quarantine selection")?;
    atomic_write(output, &data)?;
    Ok(selection.items.len())
}

pub fn apply_selection(
    db: &HealthDatabase,
    selection_path: &Path,
    input_path: &Path,
    output_path: &Path,
    confirm: bool,
) -> Result<DeletionAudit> {
    ensure!(confirm, "apply-quarantine requires --confirm");
    ensure!(selection_path.is_file(), "selection file does not exist");
    ensure!(input_path.is_file(), "input must be an existing check copy");
    if let Some(parent) = nonempty_parent(output_path) {
        fs::create_dir_all(parent).context("create output directory")?;
    }
    ensure_unoccupied(output_path, "output")?;

    let audit_path = audit_path_for(output_path);
    ensure_unoccupied(&audit_path, "deletion audit")?;
    ensure!(
        !same_path(selection_path, output_path)?,
        "selection and output must differ"
    );
    ensure!(
        !same_path(input_path, output_path)?,
        "input and output must differ"
    );
    ensure!(
        !same_path(selection_path, input_path)?,
        "selection and input must differ"
    );

    let selection: QuarantineSelection =
        serde_json::from_slice(&fs::read(selection_path).context("read quarantine selection")?)
            .context("parse quarantine selection")?;
    ensure!(
        selection.schema_version == SELECTION_SCHEMA,
        "unsupported quarantine selection schema"
    );
    // An explicitly reviewed empty selection is valid: it still produces a
    // byte-checked copy and audit record without removing anything.

    let now = Utc::now();
    let mut urls = HashSet::new();
    let mut site_keys = HashSet::new();
    let mut site_items = Vec::new();
    for item in &selection.items {
        ensure!(
            matches!(item.scope.as_str(), "bookmark" | "content" | "site"),
            "unsupported quarantine scope: {}",
            item.scope
        );
        ensure!(!item.run_id.is_empty(), "selection item has empty run_id");
        ensure!(
            !item.bookmark_id.is_empty(),
            "selection item has empty bookmark_id"
        );
        ensure!(
            !item.original_url.is_empty(),
            "selection item has empty original_url"
        );
        if item.scope == "site" {
            let site_key = item
                .bookmark_id
                .strip_prefix("site:")
                .filter(|key| !key.is_empty())
                .context("site selection item must use bookmark_id=site:<site_key>")?;
            ensure!(
                site_keys.insert(site_key.to_string()),
                "selection contains duplicate site scope: {site_key}"
            );
            ensure!(
                db.verify_quarantine_site_item(
                    &item.run_id,
                    &item.bookmark_id,
                    &item.original_url,
                    now,
                )?,
                "site selection item is stale, changed, or not confirmed_dead: {}",
                item.original_url
            );
            site_items.push((site_key.to_string(), item.original_url.clone()));
        } else {
            ensure!(
                urls.insert(item.original_url.clone()),
                "selection contains duplicate original_url"
            );
            ensure!(
                db.verify_quarantine_item(
                    &item.run_id,
                    &item.bookmark_id,
                    &item.original_url,
                    now,
                )?,
                "selection item is stale, changed, or not confirmed_dead: {}",
                item.original_url
            );
        }
    }

    let input_hash_before = hash_file(input_path)?;
    let mut bookmarks = crate::sync::import_bookmarks_from_html(
        input_path.to_str().context("input path is not UTF-8")?,
    )
    .context("read input check copy")?;
    let source_count = count_bookmarks(&bookmarks);
    let source_urls = flatten_urls(&bookmarks);
    for item in selection.items.iter().filter(|item| item.scope != "site") {
        let matches = source_urls
            .iter()
            .filter(|url| *url == &item.original_url)
            .count();
        ensure!(
            matches == 1,
            "content selection must match exactly one input bookmark, found {matches}: {}",
            item.original_url
        );
    }
    for (site_key, _) in &site_items {
        let site_urls = source_urls
            .iter()
            .filter(|url| site_key_for_url(url).as_deref() == Some(site_key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            !site_urls.is_empty(),
            "site selection item has no matching URL in input copy: {site_key}"
        );
        ensure!(
            site_urls.iter().all(|url| !urls.contains(url)),
            "site selection overlaps another selection item: {site_key}"
        );
        urls.extend(site_urls);
    }
    ensure!(
        selection
            .items
            .iter()
            .all(|item| item.scope == "site" || source_urls.contains(&item.original_url)),
        "selection does not match input copy"
    );
    let expected_removed = source_urls.iter().filter(|url| urls.contains(*url)).count();
    let stats = remove_invalid_bookmarks_preserve_structure(
        &mut bookmarks,
        &urls,
        &RemoveConfig {
            keep_empty_folders: true,
        },
    );
    ensure!(
        stats.bookmarks_removed == expected_removed,
        "selection does not match input copy: expected {}, removed {}",
        expected_removed,
        stats.bookmarks_removed
    );
    ensure!(
        source_count == stats.bookmarks_preserved + stats.bookmarks_removed,
        "source count invariant failed: source={} kept={} removed={}",
        source_count,
        stats.bookmarks_preserved,
        stats.bookmarks_removed
    );

    ensure!(
        hash_file(input_path)? == input_hash_before,
        "input check copy changed while applying selection"
    );
    let parent = nonempty_parent(output_path).unwrap_or_else(|| Path::new("."));
    // The shared exporter replaces its destination; keep it inside a private
    // directory rather than replacing a NamedTempFile's owned path.
    let staging = tempfile::tempdir_in(parent).context("create private export directory")?;
    let staged_output = staging.path().join("cleaned.html");
    crate::sync::export_bookmarks_to_html(
        &bookmarks,
        staged_output.to_str().context("output path is not UTF-8")?,
    )
    .context("write temporary cleaned copy")?;
    let mut temp_output = NamedTempFile::new_in(parent).context("stage cleaned copy")?;
    io::copy(
        &mut File::open(&staged_output).context("open temporary cleaned copy")?,
        temp_output.as_file_mut(),
    )
    .context("stage temporary cleaned copy")?;
    temp_output
        .as_file()
        .sync_all()
        .context("sync temporary cleaned copy")?;

    let input_hash_after = hash_file(input_path)?;
    if input_hash_after != input_hash_before {
        bail!("input check copy changed during apply; cleaned output discarded");
    }
    let output_bookmarks = crate::sync::import_bookmarks_from_html(
        temp_output
            .path()
            .to_str()
            .context("output path is not UTF-8")?,
    )
    .context("verify cleaned output")?;
    let output_count = count_bookmarks(&output_bookmarks);
    ensure!(
        output_count == stats.bookmarks_preserved,
        "output count invariant failed: output={} kept={}",
        output_count,
        stats.bookmarks_preserved
    );

    let audit = DeletionAudit {
        schema_version: AUDIT_SCHEMA.to_string(),
        generated_at: Utc::now(),
        selection_path: selection_path.display().to_string(),
        input_path: input_path.display().to_string(),
        output_path: output_path.display().to_string(),
        input_hash_before,
        input_hash_after,
        source_count,
        kept_count: stats.bookmarks_preserved,
        removed_count: stats.bookmarks_removed,
        output_count,
        selection_count: selection.items.len(),
        removed_items: selection.items,
    };
    let data = serde_json::to_vec_pretty(&audit).context("serialize deletion audit")?;
    let mut temp_audit = NamedTempFile::new_in(parent).context("stage deletion audit")?;
    temp_audit
        .write_all(&data)
        .context("write deletion audit")?;
    temp_audit
        .as_file()
        .sync_all()
        .context("sync deletion audit")?;
    publish_quarantine_files(temp_output, output_path, temp_audit, &audit_path)?;
    if hash_file(input_path)? != audit.input_hash_before {
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(&audit_path);
        bail!("input check copy changed before publish; output and audit discarded");
    }
    Ok(audit)
}

pub fn audit_path_for(output_path: &Path) -> PathBuf {
    let stem = output_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("cleaned-bookmarks");
    output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}.deletion-audit.json"))
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = nonempty_parent(path) {
        fs::create_dir_all(parent).context("create selection output directory")?;
    }
    crate::browser_utils::atomic_write_file(path, data).context("atomic write")
}

fn ensure_unoccupied(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("refusing to overwrite existing {label}"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {label}: {}", path.display())),
    }
}

fn publish_quarantine_files(
    output: NamedTempFile,
    output_path: &Path,
    audit: NamedTempFile,
    audit_path: &Path,
) -> Result<()> {
    // These are two separate publications, not a crash-atomic transaction.
    // No-clobber persistence is the boundary even if a target appears after preflight.
    audit
        .persist_noclobber(audit_path)
        .map_err(|error| error.error)
        .context("publish deletion audit without overwrite")?;
    if let Err(error) = output.persist_noclobber(output_path) {
        let error = error.error;
        // Only reached after this call published the audit. As before, rollback
        // assumes no concurrent writer replaces that newly published path.
        fs::remove_file(audit_path).with_context(|| {
            format!("publish cleaned copy failed ({error}); rollback deletion audit failed")
        })?;
        return Err(error).context("publish cleaned copy without overwrite");
    }
    Ok(())
}

fn nonempty_parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn same_path(left: &Path, right: &Path) -> Result<bool> {
    let left =
        fs::canonicalize(left).with_context(|| format!("canonicalize {}", left.display()))?;
    let right = if right.exists() {
        fs::canonicalize(right).with_context(|| format!("canonicalize {}", right.display()))?
    } else {
        let parent = nonempty_parent(right).unwrap_or_else(|| Path::new("."));
        fs::canonicalize(parent)
            .with_context(|| format!("canonicalize {}", parent.display()))?
            .join(right.file_name().context("output has no filename")?)
    };
    Ok(left == right)
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Hasher::new();
    io::copy(&mut file, &mut hasher).context("hash file")?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn flatten_urls(bookmarks: &[Bookmark]) -> Vec<String> {
    let mut urls = Vec::new();
    for bookmark in bookmarks {
        if bookmark.folder {
            urls.extend(flatten_urls(&bookmark.children));
        } else if let Some(url) = &bookmark.url {
            urls.push(url.clone());
        }
    }
    urls
}

fn site_key_for_url(url: &str) -> Option<String> {
    let input = crate::health::targets::BookmarkTargetInput {
        bookmark_id: "quarantine".to_string(),
        original_url: url.to_string(),
    };
    crate::health::targets::build_probe_plan(&[input], crate::health::targets::ProbeScope::Site)
        .targets
        .into_iter()
        .next()
        .map(|target| target.site_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browsers::Bookmark;
    use rusqlite::params;

    fn fixture_bookmarks() -> Vec<Bookmark> {
        vec![
            Bookmark {
                id: "keep".into(),
                title: "Keep".into(),
                url: Some("https://keep.example".into()),
                folder: false,
                children: vec![],
                date_added: None,
                date_modified: None,
            },
            Bookmark {
                id: "remove".into(),
                title: "Remove".into(),
                url: Some("https://remove.example".into()),
                folder: false,
                children: vec![],
                date_added: None,
                date_modified: None,
            },
        ]
    }

    fn confirmed_db() -> HealthDatabase {
        let db = HealthDatabase::in_memory().unwrap();
        let now = Utc::now().to_rfc3339();
        db.raw_conn_for_test()
            .execute(
                "INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,rule_version)
                 VALUES ('run-1',?1,?1,1,'standard',?2)",
                params![
                    now,
                    crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION
                ],
            )
            .unwrap();
        db.raw_conn_for_test()
            .execute(
                "INSERT INTO health_checks
                 (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict)
                 VALUES ('check-1','run-1','bm-1','https://remove.example',?1,?1,'confirmed_dead')",
                params![now],
            )
            .unwrap();
        db.raw_conn_for_test()
            .execute(
                "INSERT INTO health_summaries
                 (summary_id,bookmark_id,url,last_run_id,last_checked_at,current_verdict,total_checks,consecutive_failures)
                 VALUES ('summary-1','bm-1','https://remove.example','run-1',?1,'confirmed_dead',3,3)",
                params![now],
            )
            .unwrap();
        db
    }

    #[test]
    fn selection_contains_auditable_identity() {
        let db = confirmed_db();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("selection.json");
        assert_eq!(write_selection(&db, &path).unwrap(), 1);
        let selection: QuarantineSelection =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(selection.items[0].run_id, "run-1");
        assert_eq!(selection.items[0].bookmark_id, "bm-1");
        assert_eq!(selection.items[0].scope, "content");
    }

    #[test]
    fn apply_requires_confirm_and_preserves_input() {
        let db = confirmed_db();
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.html");
        let selection_path = temp.path().join("selection.json");
        let output = temp.path().join("cleaned.html");
        crate::sync::export_bookmarks_to_html(&fixture_bookmarks(), input.to_str().unwrap())
            .unwrap();
        write_selection(&db, &selection_path).unwrap();
        let before = fs::read(&input).unwrap();
        assert!(apply_selection(&db, &selection_path, &input, &output, false).is_err());
        let audit = apply_selection(&db, &selection_path, &input, &output, true).unwrap();
        assert_eq!(audit.source_count, audit.kept_count + audit.removed_count);
        assert_eq!(audit.output_count, audit.kept_count);
        assert_eq!(fs::read(&input).unwrap(), before);
        assert!(audit.input_hash_before == audit.input_hash_after);
        assert!(audit_path_for(&output).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&selection_path, &output, &audit_path_for(&output)] {
                assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
            }
        }
        let cleaned = crate::sync::import_bookmarks_from_html(output.to_str().unwrap()).unwrap();
        assert_eq!(count_bookmarks(&cleaned), 1);
    }

    #[test]
    fn apply_rechecks_future_and_invalid_evidence_without_publishing_output() {
        for field in ["summary", "check", "run"] {
            for timestamp in [
                (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                (Utc::now() - chrono::Duration::hours(25)).to_rfc3339(),
                "invalid-timestamp".to_string(),
            ] {
                let db = confirmed_db();
                let temp = tempfile::tempdir().unwrap();
                let input = temp.path().join("input.html");
                let selection = temp.path().join("selection.json");
                let output = temp.path().join("cleaned.html");
                crate::sync::export_bookmarks_to_html(
                    &fixture_bookmarks(),
                    input.to_str().unwrap(),
                )
                .unwrap();
                write_selection(&db, &selection).unwrap();
                let input_before = fs::read(&input).unwrap();
                let selection_before = fs::read(&selection).unwrap();
                let sql = match field {
                    "summary" => "UPDATE health_summaries SET last_checked_at = ?1",
                    "check" => "UPDATE health_checks SET finished_at = ?1",
                    _ => "UPDATE health_runs SET finished_at = ?1",
                };
                db.raw_conn_for_test()
                    .execute(sql, params![timestamp])
                    .unwrap();
                let error = apply_selection(&db, &selection, &input, &output, true).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("stale, changed, or not confirmed_dead"),
                    "{field}: {error:#}"
                );
                assert_eq!(fs::read(&input).unwrap(), input_before);
                assert_eq!(fs::read(&selection).unwrap(), selection_before);
                assert!(fs::symlink_metadata(&output).is_err());
                assert!(fs::symlink_metadata(audit_path_for(&output)).is_err());
            }
        }
    }

    #[test]
    fn stale_or_non_confirmed_selection_is_rejected() {
        let db = confirmed_db();
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.html");
        let selection_path = temp.path().join("selection.json");
        let output = temp.path().join("cleaned.html");
        crate::sync::export_bookmarks_to_html(&fixture_bookmarks(), input.to_str().unwrap())
            .unwrap();
        let selection = QuarantineSelection {
            schema_version: SELECTION_SCHEMA.into(),
            generated_at: Utc::now(),
            items: vec![QuarantineItem {
                run_id: "run-1".into(),
                bookmark_id: "bm-1".into(),
                original_url: "https://remove.example".into(),
                reason: "tampered".into(),
                scope: "site".into(),
            }],
        };
        fs::write(&selection_path, serde_json::to_vec(&selection).unwrap()).unwrap();
        assert!(apply_selection(&db, &selection_path, &input, &output, true).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn content_selection_rejects_duplicate_url_occurrences_in_input() {
        let db = confirmed_db();
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.html");
        let selection_path = temp.path().join("selection.json");
        let output = temp.path().join("cleaned.html");
        let mut bookmarks = fixture_bookmarks();
        bookmarks.push(bookmarks[1].clone());
        crate::sync::export_bookmarks_to_html(&bookmarks, input.to_str().unwrap()).unwrap();
        write_selection(&db, &selection_path).unwrap();

        let error = apply_selection(&db, &selection_path, &input, &output, true).unwrap_err();
        assert!(error.to_string().contains("exactly one"));
        assert!(!output.exists());
    }

    #[test]
    fn empty_reviewed_selection_still_writes_an_unchanged_copy() {
        let db = HealthDatabase::in_memory().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.html");
        let selection_path = temp.path().join("selection.json");
        let output = temp.path().join("cleaned.html");
        crate::sync::export_bookmarks_to_html(&fixture_bookmarks(), input.to_str().unwrap())
            .unwrap();
        let selection = QuarantineSelection {
            schema_version: SELECTION_SCHEMA.into(),
            generated_at: Utc::now(),
            items: Vec::new(),
        };
        fs::write(&selection_path, serde_json::to_vec(&selection).unwrap()).unwrap();

        let audit = apply_selection(&db, &selection_path, &input, &output, true).unwrap();
        assert_eq!(audit.removed_count, 0);
        assert_eq!(audit.source_count, audit.output_count);
        assert!(audit.input_hash_before == audit.input_hash_after);
    }

    #[test]
    fn quarantine_no_clobber_rejects_occupied_targets() {
        for occupy_audit in [false, true] {
            let db = HealthDatabase::in_memory().unwrap();
            let temp = tempfile::tempdir().unwrap();
            let input = temp.path().join("input.html");
            let selection = temp.path().join("selection.json");
            let output = temp.path().join("cleaned.html");
            let audit = audit_path_for(&output);
            crate::sync::export_bookmarks_to_html(&fixture_bookmarks(), input.to_str().unwrap())
                .unwrap();
            write_selection(&db, &selection).unwrap();
            let input_before = fs::read(&input).unwrap();
            let selection_before = fs::read(&selection).unwrap();
            let occupied = if occupy_audit { &audit } else { &output };
            fs::write(occupied, b"existing user data").unwrap();

            assert!(apply_selection(&db, &selection, &input, &output, true).is_err());
            assert_eq!(fs::read(occupied).unwrap(), b"existing user data");
            assert_eq!(fs::read(&input).unwrap(), input_before);
            assert_eq!(fs::read(&selection).unwrap(), selection_before);
            let absent = if occupy_audit { &output } else { &audit };
            assert!(fs::symlink_metadata(absent).is_err());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
        }
    }

    #[test]
    fn quarantine_no_clobber_rejects_targets_appearing_at_publication() {
        for occupy_audit in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("cleaned.html");
            let audit = audit_path_for(&output);
            let mut staged_output = NamedTempFile::new_in(temp.path()).unwrap();
            let mut staged_audit = NamedTempFile::new_in(temp.path()).unwrap();
            staged_output.write_all(b"new cleaned copy").unwrap();
            staged_audit.write_all(b"new audit").unwrap();
            ensure_unoccupied(&output, "output").unwrap();
            ensure_unoccupied(&audit, "audit").unwrap();
            let occupied = if occupy_audit { &audit } else { &output };
            fs::write(occupied, b"late user data").unwrap();

            assert!(
                publish_quarantine_files(staged_output, &output, staged_audit, &audit).is_err()
            );
            assert_eq!(fs::read(occupied).unwrap(), b"late user data");
            let absent = if occupy_audit { &output } else { &audit };
            assert!(fs::symlink_metadata(absent).is_err());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_no_clobber_rejects_symlinks_appearing_at_publication() {
        use std::os::unix::fs::symlink;

        for link_audit in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("cleaned.html");
            let audit = audit_path_for(&output);
            let missing = temp.path().join("missing-target");
            let staged_output = NamedTempFile::new_in(temp.path()).unwrap();
            let staged_audit = NamedTempFile::new_in(temp.path()).unwrap();
            ensure_unoccupied(&output, "output").unwrap();
            ensure_unoccupied(&audit, "audit").unwrap();
            let link = if link_audit { &audit } else { &output };
            symlink(&missing, link).unwrap();

            assert!(
                publish_quarantine_files(staged_output, &output, staged_audit, &audit).is_err()
            );
            assert_eq!(fs::read_link(link).unwrap(), missing);
            let absent = if link_audit { &output } else { &audit };
            assert!(fs::symlink_metadata(absent).is_err());
            assert!(fs::symlink_metadata(&missing).is_err());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_no_clobber_rejects_dangling_target_symlinks() {
        use std::os::unix::fs::symlink;

        for link_audit in [false, true] {
            let db = HealthDatabase::in_memory().unwrap();
            let temp = tempfile::tempdir().unwrap();
            let input = temp.path().join("input.html");
            let selection = temp.path().join("selection.json");
            let output = temp.path().join("cleaned.html");
            let audit = audit_path_for(&output);
            let missing = temp.path().join("missing-target");
            crate::sync::export_bookmarks_to_html(&fixture_bookmarks(), input.to_str().unwrap())
                .unwrap();
            write_selection(&db, &selection).unwrap();
            let input_before = fs::read(&input).unwrap();
            let selection_before = fs::read(&selection).unwrap();
            let link = if link_audit { &audit } else { &output };
            symlink(&missing, link).unwrap();

            assert!(apply_selection(&db, &selection, &input, &output, true).is_err());
            assert_eq!(fs::read_link(link).unwrap(), missing);
            assert_eq!(fs::read(&input).unwrap(), input_before);
            assert_eq!(fs::read(&selection).unwrap(), selection_before);
            let absent = if link_audit { &output } else { &audit };
            assert!(fs::symlink_metadata(absent).is_err());
            assert!(fs::symlink_metadata(&missing).is_err());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
        }
    }

    #[test]
    fn site_revalidation_binds_the_selected_site_identity() {
        let db = confirmed_db();
        assert!(!db
            .verify_quarantine_site_item(
                "run-1",
                "site:https://different.example:443",
                "https://remove.example",
                Utc::now(),
            )
            .unwrap());
    }
}
