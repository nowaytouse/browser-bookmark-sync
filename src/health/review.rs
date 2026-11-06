//! Native review model for exported HTML copies. Manual decisions never change health verdicts.

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ReviewEvidence {
    pub backend: String,
    pub status_code: Option<i64>,
    pub error_kind: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Clone, Default)]
struct ReviewSummary {
    verdict: String,
    last_checked_at: Option<String>,
    total_checks: i64,
    evidence: Vec<ReviewEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReviewItem {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub path: Vec<String>,
    pub verdict: String,
    pub last_checked_at: Option<String>,
    pub total_checks: i64,
    pub evidence: Vec<ReviewEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Decision {
    Keep,
    Remove,
}

pub struct ReviewDocument {
    pub input_path: PathBuf,
    pub input_hash: String,
    pub items: Vec<ReviewItem>,
    pub coverage: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ReviewAudit {
    pub schema_version: &'static str,
    pub generated_at: chrono::DateTime<Utc>,
    pub input_path: PathBuf,
    pub input_hash: String,
    pub output_path: PathBuf,
    pub source_count: usize,
    pub retained_count: usize,
    pub removed_count: usize,
    pub removed_items: Vec<ReviewItem>,
}

impl ReviewDocument {
    pub fn load(input: &Path, db: Option<&Path>, source_review: Option<&Path>) -> Result<Self> {
        ensure!(
            input.is_file(),
            "input must be an existing exported HTML file"
        );
        let input_bytes = fs::read(input).with_context(|| format!("read {}", input.display()))?;
        let html = std::str::from_utf8(&input_bytes).context("exported HTML must be UTF-8")?;
        let parsed = Html::parse_document(html);
        ensure!(
            parsed.select(&selector("a[href], dl")?).next().is_some(),
            "not a bookmark HTML export: no bookmark links or folder list found"
        );
        let coverage = source_review
            .map(|path| -> Result<Value> {
                serde_json::from_slice(
                    &fs::read(path).with_context(|| format!("read {}", path.display()))?,
                )
                .with_context(|| format!("parse {}", path.display()))
            })
            .transpose()?;
        let summaries = db.map(load_summaries).transpose()?.unwrap_or_default();
        Ok(Self {
            input_path: input.to_path_buf(),
            input_hash: blake3::hash(&input_bytes).to_hex().to_string(),
            items: collect_items(html, &summaries)?,
            coverage,
        })
    }

    pub fn export_copy(
        &self,
        decisions: &BTreeMap<usize, Decision>,
        output: &Path,
        confirm: bool,
    ) -> Result<ReviewAudit> {
        ensure!(
            confirm,
            "explicit confirmation is required to export a reviewed copy"
        );
        for id in decisions.keys() {
            ensure!(*id < self.items.len(), "unknown review item ID: {id}");
        }
        let audit_path = audit_path_for(output);
        ensure!(output != audit_path, "output and audit paths must differ");
        ensure_absent(output)?;
        ensure_absent(&audit_path)?;
        let input = fs::canonicalize(&self.input_path).context("resolve input copy")?;
        for target in [output, audit_path.as_path()] {
            let parent = target
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let resolved = fs::canonicalize(parent)
                .with_context(|| format!("resolve output directory {}", parent.display()))?
                .join(target.file_name().context("output needs a filename")?);
            ensure!(resolved != input, "output must not replace the input copy");
        }

        let input_bytes = fs::read(&self.input_path).context("re-read input copy")?;
        ensure!(
            blake3::hash(&input_bytes).to_hex().as_str() == self.input_hash,
            "input copy changed since review began"
        );
        let source = std::str::from_utf8(&input_bytes).context("exported HTML must be UTF-8")?;
        let mut document = Html::parse_document(source);
        let anchors = selector("a[href]")?;
        let headings = selector("h3")?;
        let before_headings = document
            .select(&headings)
            .map(|h| h.html())
            .collect::<Vec<_>>();
        let before = snapshot(&document, &anchors);
        ensure!(
            before.len() == self.items.len(),
            "input bookmark count changed"
        );
        let mut retained = Vec::new();
        let mut remove_ids = Vec::new();
        let mut removed_items = Vec::new();
        for (id, anchor) in document.select(&anchors).enumerate() {
            ensure!(
                anchor.value().attr("href") == Some(self.items[id].url.as_str()),
                "input bookmark order changed"
            );
            if decisions.get(&id) == Some(&Decision::Remove) {
                remove_ids.push(anchor.id());
                removed_items.push(self.items[id].clone());
            } else {
                retained.push(before[id].clone());
            }
        }
        for id in remove_ids {
            document
                .tree
                .get_mut(id)
                .context("review anchor disappeared")?
                .detach();
        }
        let output_html = document.html();
        let verified = Html::parse_document(&output_html);
        ensure!(
            snapshot(&verified, &anchors) == retained,
            "retained bookmark URL or metadata changed"
        );
        ensure!(
            verified
                .select(&headings)
                .map(|h| h.html())
                .collect::<Vec<_>>()
                == before_headings,
            "folder metadata changed"
        );
        let audit = ReviewAudit {
            schema_version: "bookmark-review-audit-v1",
            generated_at: Utc::now(),
            input_path: self.input_path.clone(),
            input_hash: self.input_hash.clone(),
            output_path: output.to_path_buf(),
            source_count: self.items.len(),
            retained_count: retained.len(),
            removed_count: removed_items.len(),
            removed_items,
        };
        ensure!(
            blake3::hash(&fs::read(&self.input_path)?).to_hex().as_str() == self.input_hash,
            "input copy changed before output write"
        );
        let audit_json = serde_json::to_vec_pretty(&audit)?;
        let output_temp = prepare_temp(output, output_html.as_bytes())?;
        let audit_temp = prepare_temp(&audit_path, &audit_json)?;
        ensure!(
            blake3::hash(&fs::read(&self.input_path)?).to_hex().as_str() == self.input_hash,
            "input copy changed before publish"
        );
        // Publish new files only. If the second publish fails, remove only the first file we made.
        let published = output_temp
            .persist_noclobber(output)
            .with_context(|| format!("publish new {}", output.display()))?;
        if let Err(error) = audit_temp.persist_noclobber(&audit_path) {
            remove_if_same_file(output, &published)?;
            return Err(error).with_context(|| format!("publish new {}", audit_path.display()));
        }
        Ok(audit)
    }
}

pub fn audit_path_for(output: &Path) -> PathBuf {
    output.with_extension("review-audit.json")
}

fn ensure_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => anyhow::bail!("refusing to overwrite existing {}", path.display()),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

fn selector(value: &str) -> Result<Selector> {
    Selector::parse(value).map_err(|error| anyhow::anyhow!("invalid selector: {error}"))
}

fn snapshot(document: &Html, anchors: &Selector) -> Vec<(String, Vec<String>)> {
    document
        .select(anchors)
        .map(|a| (a.html(), folder_path(a)))
        .collect()
}

fn prepare_temp(path: &Path, data: &[u8]) -> Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary file in {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temp.write_all(data)?;
    temp.as_file().sync_all()?;
    Ok(temp)
}

fn remove_if_same_file(path: &Path, published: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let original = published.metadata()?;
        let current = fs::symlink_metadata(path)?;
        if original.dev() == current.dev() && original.ino() == current.ino() {
            fs::remove_file(path)?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, published);
        anyhow::bail!("audit publish failed; output needs manual removal on this platform");
    }
    Ok(())
}

fn collect_items(
    html: &str,
    summaries: &HashMap<String, ReviewSummary>,
) -> Result<Vec<ReviewItem>> {
    let document = Html::parse_document(html);
    let anchors = selector("a[href]")?;
    Ok(document
        .select(&anchors)
        .enumerate()
        .map(|(id, anchor)| {
            let url = anchor.value().attr("href").unwrap_or_default().to_owned();
            let summary = summaries.get(&url).cloned().unwrap_or_default();
            ReviewItem {
                id,
                title: anchor.text().collect(),
                url,
                path: folder_path(anchor),
                verdict: if summary.verdict.is_empty() {
                    "untested".into()
                } else {
                    summary.verdict
                },
                last_checked_at: summary.last_checked_at,
                total_checks: summary.total_checks,
                evidence: summary.evidence,
            }
        })
        .collect())
}

fn folder_path(anchor: ElementRef<'_>) -> Vec<String> {
    // Netscape exports place each folder's H3 immediately before its nested DL.
    let mut path = Vec::new();
    for list in anchor.ancestors().filter_map(ElementRef::wrap) {
        if list.value().name() != "dl" {
            continue;
        }
        let mut sibling = list.prev_sibling();
        while let Some(node) = sibling {
            sibling = node.prev_sibling();
            let Some(element) = ElementRef::wrap(node) else {
                continue;
            };
            let heading = if element.value().name() == "h3" {
                Some(element)
            } else if element.value().name() == "dt" {
                element
                    .children()
                    .filter_map(ElementRef::wrap)
                    .find(|child| child.value().name() == "h3")
            } else {
                None
            };
            if let Some(heading) = heading {
                path.push(heading.text().collect());
            }
            break;
        }
    }
    path.reverse();
    path
}

fn load_summaries(path: &Path) -> Result<HashMap<String, ReviewSummary>> {
    ensure!(path.is_file(), "health DB must be an existing file");
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open read-only health DB {}", path.display()))?;
    conn.pragma_update(None, "query_only", "ON")?;
    let tx = conn.unchecked_transaction()?;
    // Only a summary whose exact content check belongs to a completed run is evidence.
    // URL matching is exact; site:* aggregate rows are never assigned to bookmarks.
    let mut stmt = tx.prepare(
        "WITH ranked AS (
            SELECT s.url, s.current_verdict, s.last_checked_at, s.total_checks,
                   c.check_id, r.rule_version,
                   ROW_NUMBER() OVER (
                     PARTITION BY s.url ORDER BY s.last_checked_at DESC, c.finished_at DESC, c.check_id DESC, s.summary_id DESC
                   ) AS rank
            FROM health_summaries s
            JOIN health_runs r ON r.run_id = s.last_run_id AND r.completed = 1
            JOIN health_checks c ON c.run_id = r.run_id
                AND c.bookmark_id = s.bookmark_id AND c.original_url = s.url
                AND c.finished_at IS NOT NULL
            WHERE s.bookmark_id NOT LIKE 'site:%'
        )
        SELECT ranked.url, ranked.current_verdict, ranked.last_checked_at,
               ranked.total_checks, a.backend, a.status_code, a.error_kind, a.finished_at,
               ranked.rule_version
        FROM ranked
        LEFT JOIN health_attempts a ON a.check_id = ranked.check_id
        WHERE ranked.rank = 1
        ORDER BY ranked.url, a.attempt_index, a.attempt_id",
    )?;
    let mut rows = stmt.query([])?;
    let mut summaries = HashMap::new();
    while let Some(row) = rows.next()? {
        let url: String = row.get(0)?;
        let verdict = row.get(1)?;
        let last_checked_at = row.get(2)?;
        let total_checks = row.get::<_, i64>(3)?.max(0);
        let recorded_rule: Option<String> = row.get(8)?;
        let current_rule = crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;
        let current_rules = recorded_rule.as_deref() == Some(current_rule);
        let summary = summaries.entry(url).or_insert_with(|| ReviewSummary {
            verdict: if current_rules { verdict } else { "needs_review".to_string() },
            last_checked_at,
            total_checks,
            evidence: if current_rules { Vec::new() } else {
                vec![ReviewEvidence {
                    backend: "rule_policy".to_string(),
                    error_kind: Some(format!("RuleVersionMismatch: recorded={}, current={current_rule}; recheck required", recorded_rule.as_deref().unwrap_or("unknown"))),
                    ..Default::default()
                }]
            },
        });
        if let Some(backend) = row.get::<_, Option<String>>(4)? {
            summary.evidence.push(ReviewEvidence {
                backend,
                status_code: row.get(5)?,
                error_kind: row.get(6)?,
                finished_at: row.get(7)?,
            });
        }
    }
    drop(rows);
    drop(stmt);
    tx.commit()?;
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn duplicate_occurrence_removal_preserves_sibling_and_folder_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("export.html");
        let output = dir.path().join("reviewed.html");
        let html = "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><H3 ADD_DATE='42'>Folder</H3><DL><p><DT><A HREF='https://x/' ADD_DATE='1'>One</A><DT><A HREF='https://x/' ADD_DATE='2'>Two</A></DL><p></DL><p>";
        fs::write(&input, html).unwrap();
        let doc = ReviewDocument::load(&input, None, None).unwrap();
        assert_eq!(doc.items[0].path, ["Folder"]);
        assert_eq!(doc.items[1].path, ["Folder"]);
        let audit = doc
            .export_copy(&BTreeMap::from([(0, Decision::Remove)]), &output, true)
            .unwrap();
        assert_eq!((audit.removed_count, audit.retained_count), (1, 1));
        let saved = fs::read_to_string(&output).unwrap();
        let parsed = Html::parse_document(&saved);
        let anchor = parsed.select(&selector("a[href]").unwrap()).next().unwrap();
        assert_eq!(anchor.value().attr("add_date"), Some("2"));
        assert_eq!(anchor.text().collect::<String>(), "Two");
        let folder = parsed.select(&selector("h3").unwrap()).next().unwrap();
        assert_eq!(folder.value().attr("add_date"), Some("42"));
        assert_eq!(fs::read_to_string(&input).unwrap(), html);
        assert!(audit_path_for(&output).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&output, &audit_path_for(&output)] {
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn rejects_stale_input_invalid_decisions_and_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("export.html");
        let output = dir.path().join("reviewed.html");
        fs::write(&input, "<a href='x'>X</a>").unwrap();
        let doc = ReviewDocument::load(&input, None, None).unwrap();
        assert!(doc.export_copy(&BTreeMap::new(), &output, false).is_err());
        assert!(doc
            .export_copy(&BTreeMap::from([(2, Decision::Remove)]), &output, true)
            .is_err());
        assert!(doc.export_copy(&BTreeMap::new(), &input, true).is_err());
        fs::write(audit_path_for(&output), "existing audit").unwrap();
        assert!(doc.export_copy(&BTreeMap::new(), &output, true).is_err());
        assert!(!output.exists());
        assert_eq!(
            fs::read_to_string(audit_path_for(&output)).unwrap(),
            "existing audit"
        );
        fs::remove_file(audit_path_for(&output)).unwrap();
        fs::write(&input, "<a href='y'>Y</a>").unwrap();
        assert!(doc.export_copy(&BTreeMap::new(), &output, true).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn read_only_db_ignores_site_and_incomplete_runs() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("health.sqlite");
        let db = crate::health::db::HealthDatabase::open(&db_path).unwrap();
        let now = Utc::now().to_rfc3339();
        for (run, completed, bookmark, url, verdict) in [
            ("complete", 1, "content:1", "https://x/", "needs_review"),
            ("site", 1, "site:x", "https://site/", "confirmed_dead"),
            (
                "partial",
                0,
                "content:2",
                "https://partial/",
                "confirmed_dead",
            ),
        ] {
            db.conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,rule_version) VALUES (?1,?2,?2,?3,'standard',?4)", params![run, now, completed, crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION]).unwrap();
            db.conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?1,?2,?3,?4,?4,?5)", params![run, bookmark, url, now, verdict]).unwrap();
            db.conn.execute("INSERT INTO health_summaries (summary_id,bookmark_id,url,last_run_id,last_checked_at,current_verdict,total_checks,consecutive_failures) VALUES (?1,?2,?3,?4,?5,?6,1,0)", params![run, bookmark, url, run, now, verdict]).unwrap();
        }
        db.conn.execute(
            "INSERT INTO health_attempts (attempt_id,check_id,backend,backend_version,route,attempt_index,actual_request,cache_hit,method,status_code,error_kind,initial_url,redirect_count,response_time_ms,confidence,finished_at) VALUES ('attempt','complete','curl','1','direct',0,'yes',0,'GET',403,NULL,'https://x/',0,1,1,?1)",
            params![now],
        ).unwrap();
        drop(db);
        let before = fs::read(&db_path).unwrap();
        let summaries = load_summaries(&db_path).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries["https://x/"].evidence[0].backend, "curl");
        assert_eq!(fs::read(&db_path).unwrap(), before);
    }

    #[test]
    fn accuracy_review_downgrades_stale_rules_without_changing_the_db() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.html");
        let db_path = dir.path().join("health.sqlite");
        fs::write(&input, "<DL><DT><A HREF='https://old.example/'>Old</A><DT><A HREF='https://new.example/'>New</A></DL>").unwrap();
        let db = crate::health::db::HealthDatabase::open(&db_path).unwrap();
        let now = Utc::now().to_rfc3339();
        for (run, url, rule, verdict) in [
            ("old", "https://old.example/", "8", "confirmed_dead"),
            (
                "new",
                "https://new.example/",
                crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION,
                "live",
            ),
        ] {
            db.conn.execute("INSERT INTO health_runs (run_id,started_at,finished_at,completed,mode,rule_version) VALUES (?1,?2,?2,1,'standard',?3)", params![run, now, rule]).unwrap();
            db.conn.execute("INSERT INTO health_checks (check_id,run_id,bookmark_id,original_url,started_at,finished_at,run_verdict) VALUES (?1,?1,?1,?2,?3,?3,?4)", params![run, url, now, verdict]).unwrap();
            db.conn.execute("INSERT INTO health_summaries (summary_id,bookmark_id,url,last_run_id,last_checked_at,current_verdict,total_checks,consecutive_failures) VALUES (?1,?1,?2,?1,?3,?4,1,0)", params![run, url, now, verdict]).unwrap();
        }
        drop(db);
        let before = fs::read(&db_path).unwrap();
        let doc = ReviewDocument::load(&input, Some(&db_path), None).unwrap();
        assert_eq!(doc.items[0].verdict, "needs_review");
        assert!(doc.items[0].evidence.iter().any(|e| e
            .error_kind
            .as_deref()
            .is_some_and(|error| error.starts_with("RuleVersionMismatch:"))));
        assert_eq!(doc.items[1].verdict, "live");
        assert_eq!(fs::read(&db_path).unwrap(), before);
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("export.html");
        fs::write(&input, [b'<', 0xff, b'>']).unwrap();
        assert!(ReviewDocument::load(&input, None, None).is_err());
        fs::write(&input, "ordinary text, not a bookmark export").unwrap();
        assert!(ReviewDocument::load(&input, None, None).is_err());
        fs::write(&input, "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p></DL>").unwrap();
        assert!(ReviewDocument::load(&input, None, None)
            .unwrap()
            .items
            .is_empty());
    }

    #[test]
    fn nested_netscape_copy_removes_only_selected_occurrences() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("input.html");
        let output = dir.path().join("reviewed.html");
        let original = include_bytes!("../../tests/fixtures/review-bookmarks.html");
        fs::write(&input, original)?;
        let doc = ReviewDocument::load(&input, None, None)?;
        assert_eq!(doc.items.len(), 5);
        assert_eq!(doc.items[3].path, ["工作 & 资料", "子文件夹"]);
        let audit = doc.export_copy(
            &BTreeMap::from([(1, Decision::Remove), (3, Decision::Remove)]),
            &output,
            true,
        )?;
        let saved = ReviewDocument::load(&output, None, None)?;
        assert_eq!((audit.removed_count, saved.items.len()), (2, 3));
        assert_eq!(saved.items[1].title, "重复条目 B");
        assert_eq!(saved.items[0].url, "https://example.com/a?x=1&y=2");
        assert_eq!(fs::read(input)?, original);
        Ok(())
    }
}
