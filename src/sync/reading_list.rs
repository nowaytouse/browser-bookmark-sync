mod chromium;

use anyhow::{Context, Result};
use std::path::Path;

use crate::browser_utils::create_profile_snapshot;
use crate::browsers::{Bookmark, ReadingListItem};
use crate::data_types::models::{BrowserKind, BrowserProfile};
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
    let temp = tempfile::tempdir().context("create isolated reading-list snapshots")?;
    let mut bookmarks = Vec::new();
    let mut reports = Vec::new();
    for installation in BookmarkCollector::discover_installations()? {
        if !selected(&installation.name, browser_names) {
            continue;
        }
        for profile in BookmarkCollector::discover_profiles(&installation)? {
            let source = format!(
                "{}:{}:{}",
                installation.name,
                profile.id,
                profile.path.display()
            );
            match &installation.kind {
                BrowserKind::Safari => {
                    let items = safari_items(&profile, temp.path()).with_context(|| {
                        format!("read Safari reading list from {:?}", profile.path)
                    })?;
                    let count = items.len();
                    bookmarks.extend(items.into_iter().map(|item| item_bookmark(&source, item)));
                    reports.push(report(
                        &installation.name,
                        &profile.name,
                        &profile.path,
                        "complete",
                        count,
                    ));
                }
                BrowserKind::Chromium(_) => {
                    let store = profile.path.join("Sync Data/LevelDB");
                    let metadata = match std::fs::metadata(&store) {
                        Ok(metadata) => Some(metadata),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("inspect reading-list store {store:?}"))
                        }
                    };
                    if metadata.is_none() {
                        reports.push(report(
                            &installation.name,
                            &profile.name,
                            &store,
                            "not_present",
                            0,
                        ));
                        continue;
                    }
                    anyhow::ensure!(
                        metadata.is_some_and(|metadata| metadata.is_dir()),
                        "Reading-list store is not a directory: {store:?}"
                    );
                    let items = chromium::read_store(&store)
                        .with_context(|| format!("read Chromium reading list from {store:?}"))?;
                    let count = items.len();
                    bookmarks.extend(items.into_iter().map(|item| item_bookmark(&source, item)));
                    reports.push(report(
                        &installation.name,
                        &profile.name,
                        &store,
                        "complete",
                        count,
                    ));
                }
                BrowserKind::Firefox(_) => reports.push(report(
                    &installation.name,
                    &profile.name,
                    &profile.path,
                    "unsupported",
                    0,
                )),
            }
        }
    }
    Ok((bookmarks, reports))
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
}
