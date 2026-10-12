mod chromium;

use anyhow::{Context, Result};
use std::path::Path;

use crate::browser_utils::create_profile_snapshot;
use crate::browsers::{Bookmark, ReadingListItem};
use crate::data_types::models::{BrowserInstallation, BrowserKind, BrowserProfile};
use crate::sync::collector::BookmarkCollector;

fn item_bookmark(source: &str, item: ReadingListItem) -> Bookmark {
    let id = format!(
        "rl-{}",
        blake3::hash(format!("{source}\n{}", item.url).as_bytes())
    );
    Bookmark {
        id,
        title: item.title,
        url: Some(item.url),
        folder: false,
        children: Vec::new(),
        date_added: item.date_added,
        date_modified: None,
    }
}

fn report(
    browser: &str,
    profile: &str,
    path: &Path,
    status: &str,
    count: usize,
) -> serde_json::Value {
    serde_json::json!({
        "source_type": "reading_list",
        "browser": browser,
        "profile": profile,
        "path": path,
        "status": status,
        "bookmark_count": count,
    })
}

fn selected(name: &str, browser_names: &str) -> bool {
    browser_names
        .split(',')
        .map(|value| value.trim().to_ascii_lowercase())
        .any(|value| {
            value == "all" || (!value.is_empty() && name.to_ascii_lowercase().contains(&value))
        })
}

fn safari_items(profile: &BrowserProfile, snapshot_dir: &Path) -> Result<Vec<ReadingListItem>> {
    let snapshot = create_profile_snapshot(profile, snapshot_dir)?;
    let plist = snapshot.snapshot_path.join("Bookmarks.plist");
    let data = std::fs::read(&plist).with_context(|| format!("read Safari snapshot {plist:?}"))?;
    let value: plist::Value = plist::from_bytes(&data)?;
    crate::browsers::parse_safari_reading_list(&value)
}

pub fn collect_for_export(browser_names: &str) -> Result<(Vec<Bookmark>, Vec<serde_json::Value>)> {
    let (installations, issues) = BookmarkCollector::discover_installations_with_report()?;
    collect_installed_lists(browser_names, installations, issues, false)
}

/// Interactive import keeps successful sources, with unread sources explicitly reported.
pub fn collect_for_import(browser_names: &str) -> Result<(Vec<Bookmark>, Vec<serde_json::Value>)> {
    let (installations, issues) = BookmarkCollector::discover_installations_with_report()?;
    collect_installed_lists(browser_names, installations, issues, true)
}

fn failed_report(
    installation: &BrowserInstallation,
    profile: &str,
    path: &Path,
    error: &anyhow::Error,
) -> serde_json::Value {
    serde_json::json!({
        "source_type": "reading_list",
        "browser": installation.name,
        "profile": profile,
        "path": path,
        "status": "failed",
        "error": format!("{error:#}"),
    })
}

fn collect_installed_lists(
    browser_names: &str,
    installations: Vec<BrowserInstallation>,
    mut reports: Vec<serde_json::Value>,
    allow_partial: bool,
) -> Result<(Vec<Bookmark>, Vec<serde_json::Value>)> {
    let temp = tempfile::tempdir().context("create isolated reading-list snapshots")?;
    let mut bookmarks = Vec::new();
    for installation in installations {
        if !selected(&installation.name, browser_names) {
            continue;
        }
        let profiles = match BookmarkCollector::discover_reading_list_profiles(&installation) {
            Ok(profiles) => profiles,
            Err(error) if allow_partial => {
                reports.push(failed_report(
                    &installation,
                    "",
                    &installation.root_path,
                    &error,
                ));
                continue;
            }
            Err(error) => return Err(error),
        };
        for profile in profiles {
            let collection_path = match installation.kind {
                BrowserKind::Chromium(_) => profile.path.join("Sync Data/LevelDB"),
                _ => profile.path.clone(),
            };
            let source = format!(
                "{}:{}:{}",
                installation.name,
                profile.id,
                profile.path.display()
            );
            match read_profile(&installation, &profile, temp.path()) {
                Ok((items, status)) => {
                    let count = items.len();
                    bookmarks.extend(items.into_iter().map(|item| item_bookmark(&source, item)));
                    reports.push(report(
                        &installation.name,
                        &profile.name,
                        &collection_path,
                        status,
                        count,
                    ));
                }
                Err(error) if allow_partial => {
                    reports.push(failed_report(
                        &installation,
                        &profile.name,
                        &collection_path,
                        &error,
                    ));
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok((bookmarks, reports))
}

fn read_profile(
    installation: &BrowserInstallation,
    profile: &BrowserProfile,
    snapshot_dir: &Path,
) -> Result<(Vec<ReadingListItem>, &'static str)> {
    match installation.kind {
        BrowserKind::Safari => safari_items(profile, snapshot_dir)
            .with_context(|| format!("read Safari reading list from {:?}", profile.path))
            .map(|items| (items, "complete")),
        BrowserKind::Chromium(_) => {
            let store = profile.path.join("Sync Data/LevelDB");
            let metadata = match std::fs::metadata(&store) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok((Vec::new(), "not_present"))
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("inspect reading-list store {store:?}"))
                }
            };
            anyhow::ensure!(
                metadata.is_dir(),
                "Reading-list store is not a directory: {store:?}"
            );
            chromium::read_store(&store)
                .with_context(|| format!("read Chromium reading list from {store:?}"))
                .map(|items| (items, "complete"))
        }
        BrowserKind::Firefox(_) => Ok((Vec::new(), "unsupported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_list_ids_include_source_and_url() {
        let item = ReadingListItem {
            url: "https://fixture.invalid/read".into(),
            title: "fixture".into(),
            date_added: None,
        };
        let first = item_bookmark("Safari:default:/one", item.clone());
        let second = item_bookmark("Safari:default:/two", item.clone());
        assert_ne!(first.id, second.id);
        let mut same_length = item;
        same_length.url = "https://fixture.invalid/next".into();
        assert_eq!(first.url.as_ref().unwrap().len(), same_length.url.len());
        assert_ne!(
            first.id,
            item_bookmark("Safari:default:/one", same_length).id
        );
        assert!(selected("Safari", "SAFARI,"));
        assert!(!selected("Chrome", "Safari,"));
    }

    #[test]
    fn interactive_import_reports_failed_source_and_retains_successful_reading_list() -> Result<()>
    {
        let temp = tempfile::tempdir()?;
        let broken = temp.path().join("Broken/Bookmarks.plist");
        std::fs::create_dir_all(broken.parent().unwrap())?;
        std::fs::write(&broken, b"not a plist")?;
        let valid = temp.path().join("Safari/Bookmarks.plist");
        std::fs::create_dir_all(valid.parent().unwrap())?;
        let value: plist::Value = plist::from_bytes(br#"<?xml version="1.0"?><plist version="1.0"><dict><key>Children</key><array><dict><key>Title</key><string>com.apple.ReadingList</string><key>Children</key><array><dict><key>ReadingList</key><dict/><key>URLString</key><string>https://fixture.invalid/read</string><key>URIDictionary</key><dict><key>title</key><string>Reading fixture</string></dict></dict></array></dict></array></dict></plist>"#)?;
        plist::to_file_xml(&valid, &value)?;
        let installations = vec![
            BrowserInstallation {
                name: "Broken Safari".into(),
                kind: BrowserKind::Safari,
                channel: None,
                root_path: broken,
            },
            BrowserInstallation {
                name: "Safari".into(),
                kind: BrowserKind::Safari,
                channel: None,
                root_path: valid,
            },
        ];
        assert!(collect_installed_lists("all", installations.clone(), Vec::new(), false).is_err());
        let (bookmarks, reports) = collect_installed_lists("all", installations, Vec::new(), true)?;
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(
            bookmarks[0].url.as_deref(),
            Some("https://fixture.invalid/read")
        );
        let failed = reports
            .iter()
            .find(|report| report["status"] == "failed")
            .unwrap();
        assert!(failed.get("bookmark_count").is_none());
        assert!(failed["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()));
        assert!(reports
            .iter()
            .any(|report| report["status"] == "complete" && report["bookmark_count"] == 1));
        Ok(())
    }
}
