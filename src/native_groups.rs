//! Filtered groups retain original occurrence IDs and never make cleanup decisions.
use crate::health::review::ReviewItem;
use crate::native_preferences::GroupChoice;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct BookmarkGroup {
    pub key: String,
    pub label: String,
    pub ids: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupRow {
    Header(usize),
    Bookmark(usize),
}

pub fn groups(items: &[ReviewItem], filtered: &[usize], choice: GroupChoice) -> Vec<BookmarkGroup> {
    let mut groups: BTreeMap<String, BookmarkGroup> = BTreeMap::new();
    for &id in filtered {
        let Some(item) = items.get(id) else {
            continue;
        };
        let (key, label) = match choice {
            GroupChoice::Folder => (format!("folder:{:?}", item.path), item.path.join(" / ")),
            GroupChoice::Website => {
                let host = reqwest::Url::parse(&item.url)
                    .ok()
                    .filter(|url| matches!(url.scheme(), "http" | "https"))
                    .and_then(|url| url.host_str().map(str::to_lowercase));
                let label = host.unwrap_or_else(|| "non_web".into());
                (format!("website:{label}"), label)
            }
            GroupChoice::Status => (format!("status:{}", item.verdict), item.verdict.clone()),
            GroupChoice::None => ("all".into(), String::new()),
        };
        groups
            .entry(key.clone())
            .or_insert_with(|| BookmarkGroup {
                key,
                label,
                ids: Vec::new(),
            })
            .ids
            .push(id);
    }
    groups.into_values().collect()
}

pub fn rows(
    groups: &[BookmarkGroup],
    collapsed: &BTreeSet<String>,
    choice: GroupChoice,
) -> Vec<GroupRow> {
    let mut rows = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        if choice != GroupChoice::None {
            rows.push(GroupRow::Header(index));
        }
        if choice == GroupChoice::None || !collapsed.contains(&group.key) {
            rows.extend(group.ids.iter().copied().map(GroupRow::Bookmark));
        }
    }
    rows
}

pub fn select_group(selected: &mut BTreeSet<usize>, ids: &[usize], select: bool) {
    for &id in ids {
        if select {
            selected.insert(id);
        } else {
            selected.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: usize, path: &[&str], url: &str) -> ReviewItem {
        ReviewItem {
            id,
            title: format!("示例 {id}"),
            url: url.into(),
            path: path.iter().map(|s| (*s).into()).collect(),
            verdict: "untested".into(),
            last_checked_at: None,
            total_checks: 0,
            evidence: Vec::new(),
        }
    }
    #[test]
    fn folder_path_boundaries_and_duplicate_occurrences_are_preserved() {
        let items = vec![
            item(0, &["A / B"], "https://example.test/one"),
            item(1, &["A", "B"], "https://example.test/one"),
            item(2, &["A", "B"], "mailto:test@example.test"),
        ];
        let grouped = groups(&items, &[0, 1, 2], GroupChoice::Folder);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped.iter().map(|g| g.ids.len()).sum::<usize>(), 3);
        assert_ne!(grouped[0].key, grouped[1].key);
    }
    #[test]
    fn status_groups_and_flat_rows_keep_the_complete_occurrence_set() {
        let mut items = vec![
            item(0, &[], "https://example.test/one"),
            item(1, &[], "https://example.test/two"),
        ];
        items[1].verdict = "needs_review".into();
        let status = groups(&items, &[0, 1], GroupChoice::Status);
        assert_eq!(status.len(), 2);
        let plain = groups(&items, &[0, 1], GroupChoice::None);
        assert_eq!(
            rows(&plain, &BTreeSet::from(["all".into()]), GroupChoice::None),
            [GroupRow::Bookmark(0), GroupRow::Bookmark(1)]
        );
    }

    #[test]
    fn filtered_group_selection_and_collapsing_do_not_touch_hidden_occurrences() {
        let items = vec![
            item(0, &["阅读"], "https://EXAMPLE.test/one"),
            item(1, &["工作"], "https://example.test/two"),
            item(2, &[], "file:///tmp/synthetic.html"),
        ];
        let grouped = groups(&items, &[0, 2], GroupChoice::Website);
        assert_eq!(grouped[0].label, "example.test");
        assert_eq!(
            groups(&items, &[0, 1, 2], GroupChoice::Website)[0].ids,
            [0, 1]
        );
        let mut selected = BTreeSet::from([1]);
        select_group(&mut selected, &grouped[0].ids, true);
        assert_eq!(selected, BTreeSet::from([0, 1]));
        select_group(&mut selected, &grouped[0].ids, false);
        assert_eq!(selected, BTreeSet::from([1]));
        let collapsed = BTreeSet::from([grouped[0].key.clone()]);
        let visible = rows(&grouped, &collapsed, GroupChoice::Website);
        assert!(!visible.contains(&GroupRow::Bookmark(0)));
        assert!(visible.contains(&GroupRow::Bookmark(2)));
        assert_eq!(grouped[0].ids, [0]);
    }
}
