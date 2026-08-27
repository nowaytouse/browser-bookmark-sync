//! Deterministic Exact URL Deduplication Engine

use crate::data_types::canonical::{
    CanonicalBookmark, DuplicateCsvRecord, DuplicateGroupReport, SelectionReason,
};
use crate::data_types::models::{BookmarkNode, BookmarkOccurrence, BookmarkRootKind, BookmarkTree};
use std::collections::HashMap;

pub struct DedupeEngine;

impl DedupeEngine {
    /// Perform deterministic exact deduplication over a harvested `BookmarkTree`
    pub fn deduplicate_tree(
        tree: &BookmarkTree,
        source_priority: &[String],
    ) -> (
        Vec<CanonicalBookmark>,
        Vec<DuplicateGroupReport>,
        Vec<DuplicateCsvRecord>,
        usize,
        usize,
    ) {
        let mut occurrences = Vec::new();
        for root in &tree.roots {
            Self::flatten_node_occurrences(root, &mut occurrences);
        }

        let total_raw = occurrences.len();

        // Exact means byte-for-byte equality; do not silently normalize user data.
        let mut url_groups: HashMap<String, Vec<BookmarkOccurrence>> = HashMap::new();
        for occ in occurrences {
            let key = occ.raw_url.clone();
            url_groups.entry(key).or_default().push(occ);
        }

        let mut canonical_bookmarks = Vec::new();
        let mut group_reports = Vec::new();
        let mut csv_records = Vec::new();

        let mut group_counter = 0;
        let mut total_duplicates_count = 0;

        // Sort keys to guarantee deterministic processing order regardless of HashMap layout
        let mut sorted_keys: Vec<String> = url_groups.keys().cloned().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            let Some(mut group_occurrences) = url_groups.remove(&key) else {
                tracing::warn!("去重过程中未找到预排序的 URL 键 '{}'，已跳过", key);
                continue;
            };
            group_counter += 1;
            let group_id = format!("GRP-{group_counter:06}");

            if group_occurrences.len() == 1 {
                let Some(single) = group_occurrences.pop() else {
                    tracing::warn!("去重分组 '{}' 长度为 1 但无法弹出元素，已跳过", group_id);
                    continue;
                };
                let canonical = CanonicalBookmark {
                    canonical_id: group_id.clone(),
                    exact_url: key.clone(),
                    primary: single.clone(),
                    duplicates: vec![],
                    selection_reason: vec![SelectionReason::DeterministicFallback],
                };
                canonical_bookmarks.push(canonical);

                csv_records.push(DuplicateCsvRecord {
                    group_id,
                    exact_url: key,
                    action: "KEPT".into(),
                    browser: single.source.browser.to_string(),
                    profile: single.source.profile_id.clone(),
                    root: single.root_kind.to_string(),
                    folder_path: single.folder_path.join("/"),
                    title: single.title.clone(),
                    date_added: single
                        .date_added
                        .map_or_else(|| "N/A".to_string(), |d| d.to_rfc3339()),
                    date_modified: single
                        .date_modified
                        .map_or_else(|| "N/A".to_string(), |d| d.to_rfc3339()),
                    original_position: single.original_position,
                    selection_reason: "sole_occurrence".into(),
                });
            } else {
                total_duplicates_count += group_occurrences.len() - 1;

                // Pick primary occurrence according to deterministic tie-breakers
                let (primary_idx, reasons) =
                    Self::select_primary_index(&group_occurrences, source_priority);

                let primary = group_occurrences.remove(primary_idx);
                let duplicates = group_occurrences;

                let canonical = CanonicalBookmark {
                    canonical_id: group_id.clone(),
                    exact_url: key.clone(),
                    primary: primary.clone(),
                    duplicates: duplicates.clone(),
                    selection_reason: reasons.clone(),
                };

                let reason_strs: Vec<String> = reasons.iter().map(ToString::to_string).collect();

                group_reports.push(DuplicateGroupReport {
                    group_id: group_id.clone(),
                    exact_url: key.clone(),
                    primary_title: primary.title.clone(),
                    primary_source: primary.source.browser.to_string(),
                    primary_folder_path: primary.folder_path.join("/"),
                    duplicate_count: duplicates.len(),
                    duplicates: duplicates.clone(),
                    selection_reasons: reason_strs.clone(),
                });

                // Primary CSV row
                csv_records.push(DuplicateCsvRecord {
                    group_id: group_id.clone(),
                    exact_url: key.clone(),
                    action: "KEPT".into(),
                    browser: primary.source.browser.to_string(),
                    profile: primary.source.profile_id.clone(),
                    root: primary.root_kind.to_string(),
                    folder_path: primary.folder_path.join("/"),
                    title: primary.title.clone(),
                    date_added: primary
                        .date_added
                        .map_or_else(|| "N/A".to_string(), |d| d.to_rfc3339()),
                    date_modified: primary
                        .date_modified
                        .map_or_else(|| "N/A".to_string(), |d| d.to_rfc3339()),
                    original_position: primary.original_position,
                    selection_reason: reason_strs.join(";"),
                });

                // Duplicates CSV rows
                for dup in &duplicates {
                    csv_records.push(DuplicateCsvRecord {
                        group_id: group_id.clone(),
                        exact_url: key.clone(),
                        action: "MERGED".into(),
                        browser: dup.source.browser.to_string(),
                        profile: dup.source.profile_id.clone(),
                        root: dup.root_kind.to_string(),
                        folder_path: dup.folder_path.join("/"),
                        title: dup.title.clone(),
                        date_added: dup
                            .date_added
                            .map_or_else(|| "N/A".to_string(), |d| d.to_rfc3339()),
                        date_modified: dup
                            .date_modified
                            .map_or_else(|| "N/A".to_string(), |d| d.to_rfc3339()),
                        original_position: dup.original_position,
                        selection_reason: format!("duplicate_of_{}", primary.id),
                    });
                }

                canonical_bookmarks.push(canonical);
            }
        }

        (
            canonical_bookmarks,
            group_reports,
            csv_records,
            total_raw,
            total_duplicates_count,
        )
    }

    fn flatten_node_occurrences(node: &BookmarkNode, out: &mut Vec<BookmarkOccurrence>) {
        match node {
            BookmarkNode::Bookmark(occ) => out.push(occ.clone()),
            BookmarkNode::Folder(f) => {
                for child in &f.children {
                    Self::flatten_node_occurrences(child, out);
                }
            }
            BookmarkNode::Separator(_) => {}
        }
    }

    fn select_primary_index(
        items: &[BookmarkOccurrence],
        source_priority: &[String],
    ) -> (usize, Vec<SelectionReason>) {
        let mut best_idx = 0;
        let mut reasons = vec![SelectionReason::DeterministicFallback];

        for i in 1..items.len() {
            let candidate = &items[i];
            let current_best = &items[best_idx];

            // 1. Source priority check
            if !source_priority.is_empty() {
                let cand_p = source_priority
                    .iter()
                    .position(|s| candidate.source.browser.to_string().contains(s));
                let best_p = source_priority
                    .iter()
                    .position(|s| current_best.source.browser.to_string().contains(s));

                match (cand_p, best_p) {
                    (Some(cp), Some(bp)) if cp < bp => {
                        best_idx = i;
                        reasons = vec![SelectionReason::PreferredSource];
                        continue;
                    }
                    (Some(_), None) => {
                        best_idx = i;
                        reasons = vec![SelectionReason::PreferredSource];
                        continue;
                    }
                    (None, Some(_)) => continue,
                    _ => {}
                }
            }

            // 2. Root kind priority check (BookmarkBar > BookmarkMenu > Other)
            let root_rank = |rk: &BookmarkRootKind| match rk {
                BookmarkRootKind::BookmarkBar => 1,
                BookmarkRootKind::BookmarkMenu => 2,
                BookmarkRootKind::Toolbar => 3,
                BookmarkRootKind::OtherBookmarks => 4,
                BookmarkRootKind::MobileBookmarks => 5,
                _ => 6,
            };

            if root_rank(&candidate.root_kind) < root_rank(&current_best.root_kind) {
                best_idx = i;
                reasons = vec![SelectionReason::PreferredRootKind];
                continue;
            } else if root_rank(&candidate.root_kind) > root_rank(&current_best.root_kind) {
                continue;
            }

            // 3. Deeper folder path check
            if candidate.folder_path.len() > current_best.folder_path.len() {
                best_idx = i;
                reasons = vec![SelectionReason::DeeperFolderPath];
                continue;
            } else if candidate.folder_path.len() < current_best.folder_path.len() {
                continue;
            }

            // 4. Earliest date added check
            if let (Some(c_date), Some(b_date)) = (candidate.date_added, current_best.date_added) {
                if c_date < b_date {
                    best_idx = i;
                    reasons = vec![SelectionReason::EarliestDateAdded];
                    continue;
                }
            }

            // Stable final tie-breaker independent of input traversal order.
            let stable_key = |item: &BookmarkOccurrence| {
                (
                    item.source.browser.to_string(),
                    item.source.browser_channel.clone(),
                    item.source.profile_id.clone(),
                    item.folder_path.clone(),
                    item.original_position,
                    item.id.clone(),
                )
            };
            if stable_key(candidate) < stable_key(current_best) {
                best_idx = i;
                reasons = vec![SelectionReason::DeterministicFallback];
            }
        }

        (best_idx, reasons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_types::models::{
        BookmarkNode, BookmarkOccurrence, BookmarkRootKind, BookmarkTree, BrowserKind,
        SourceDescriptor,
    };
    use chrono::Utc;
    use std::path::PathBuf;

    fn make_sample_occurrence(url: &str, title: &str, browser_name: &str) -> BookmarkOccurrence {
        BookmarkOccurrence {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
            raw_url: url.into(),
            source: SourceDescriptor {
                browser: BrowserKind::Chromium(browser_name.into()),
                browser_channel: None,
                profile_id: "Default".into(),
                profile_name: "Default Profile".into(),
                source_path: PathBuf::from("/tmp/test"),
            },
            root_kind: BookmarkRootKind::BookmarkBar,
            folder_path: vec!["Root".into()],
            original_position: 0,
            date_added: Some(Utc::now()),
            date_modified: Some(Utc::now()),
            metadata: Default::default(),
        }
    }

    #[test]
    fn test_dedupe_invariant() {
        let occ1 = make_sample_occurrence("https://example.com", "Example 1", "Chrome");
        let occ2 = make_sample_occurrence("https://example.com", "Example 2", "Brave");
        let occ3 = make_sample_occurrence("https://rust-lang.org", "Rust", "Firefox");

        let tree = BookmarkTree {
            sources: vec![],
            roots: vec![
                BookmarkNode::Bookmark(occ1),
                BookmarkNode::Bookmark(occ2),
                BookmarkNode::Bookmark(occ3),
            ],
            harvested_at: Utc::now(),
        };

        let (canonicals, _reports, _csvs, raw_count, dup_count) =
            DedupeEngine::deduplicate_tree(&tree, &["Chrome".into()]);

        assert_eq!(raw_count, 3);
        assert_eq!(canonicals.len(), 2);
        assert_eq!(dup_count, 1);
        assert_eq!(raw_count, canonicals.len() + dup_count);

        let example_canonical = canonicals
            .iter()
            .find(|c| c.exact_url == "https://example.com")
            .unwrap();
        assert_eq!(
            example_canonical.primary.source.browser.to_string(),
            "Chromium(Chrome)"
        );
        assert_eq!(example_canonical.duplicates.len(), 1);
    }

    #[test]
    fn primary_selection_honors_priority_and_is_order_independent() {
        let mut brave = make_sample_occurrence("https://example.com", "Same", "Brave");
        let mut chrome = make_sample_occurrence("https://example.com", "Same", "Chrome");
        brave.id = "b".into();
        chrome.id = "a".into();
        chrome.date_added = brave.date_added;
        chrome.date_modified = brave.date_modified;

        let forward = vec![brave.clone(), chrome.clone()];
        let reverse = vec![chrome.clone(), brave.clone()];
        let (preferred, _) = DedupeEngine::select_primary_index(&forward, &["Chrome".into()]);
        assert_eq!(forward[preferred].id, "a");

        let (forward_index, _) = DedupeEngine::select_primary_index(&forward, &[]);
        let (reverse_index, _) = DedupeEngine::select_primary_index(&reverse, &[]);
        assert_eq!(forward[forward_index].id, reverse[reverse_index].id);
    }

    #[test]
    fn exact_dedupe_does_not_merge_whitespace_distinct_urls() {
        let tree = BookmarkTree {
            sources: vec![],
            roots: vec![
                BookmarkNode::Bookmark(make_sample_occurrence(
                    "https://example.com",
                    "plain",
                    "Chrome",
                )),
                BookmarkNode::Bookmark(make_sample_occurrence(
                    " https://example.com",
                    "leading-space",
                    "Chrome",
                )),
            ],
            harvested_at: Utc::now(),
        };
        let (canonicals, _, _, raw, duplicates) = DedupeEngine::deduplicate_tree(&tree, &[]);
        assert_eq!((raw, canonicals.len(), duplicates), (2, 2, 0));
    }
}
