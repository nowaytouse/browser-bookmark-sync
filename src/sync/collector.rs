//! Comprehensive Browser and Profile Harvester with Snapshot Isolation

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::browser_utils::create_profile_snapshot;
use crate::data_types::manifest::{
    BrowserInstallationSummary, BrowserProfileSummary, InventoryReport, RunManifest,
};
use crate::data_types::models::{
    BookmarkNode, BookmarkOccurrence, BookmarkRootKind, BookmarkTree, BrowserInstallation,
    BrowserKind, BrowserProfile, FolderNode, ProfileSnapshot, SeparatorNode, SourceDescriptor,
};

/// High-level harvester struct
pub struct BookmarkCollector;

impl BookmarkCollector {
    /// Discover all browser installations on macOS
    pub fn discover_installations() -> Vec<BrowserInstallation> {
        let home = match std::env::var("HOME") {
            Ok(h) => h,
            Err(_) => return vec![],
        };

        let candidates = vec![
            (
                "Google Chrome",
                BrowserKind::Chromium("Chrome".into()),
                None,
                format!("{}/Library/Application Support/Google/Chrome", home),
            ),
            (
                "Google Chrome Beta",
                BrowserKind::Chromium("Chrome Beta".into()),
                Some("Beta".into()),
                format!("{}/Library/Application Support/Google/Chrome Beta", home),
            ),
            (
                "Google Chrome Dev",
                BrowserKind::Chromium("Chrome Dev".into()),
                Some("Dev".into()),
                format!("{}/Library/Application Support/Google/Chrome Dev", home),
            ),
            (
                "Google Chrome Canary",
                BrowserKind::Chromium("Chrome Canary".into()),
                Some("Canary".into()),
                format!("{}/Library/Application Support/Google/Chrome Canary", home),
            ),
            (
                "Brave",
                BrowserKind::Chromium("Brave".into()),
                None,
                format!(
                    "{}/Library/Application Support/BraveSoftware/Brave-Browser",
                    home
                ),
            ),
            (
                "Brave Beta",
                BrowserKind::Chromium("Brave Beta".into()),
                Some("Beta".into()),
                format!(
                    "{}/Library/Application Support/BraveSoftware/Brave-Browser-Beta",
                    home
                ),
            ),
            (
                "Brave Nightly",
                BrowserKind::Chromium("Brave Nightly".into()),
                Some("Nightly".into()),
                format!(
                    "{}/Library/Application Support/BraveSoftware/Brave-Browser-Nightly",
                    home
                ),
            ),
            (
                "Microsoft Edge",
                BrowserKind::Chromium("Edge".into()),
                None,
                format!("{}/Library/Application Support/Microsoft Edge", home),
            ),
            (
                "Microsoft Edge Beta",
                BrowserKind::Chromium("Edge Beta".into()),
                Some("Beta".into()),
                format!("{}/Library/Application Support/Microsoft Edge Beta", home),
            ),
            (
                "Microsoft Edge Dev",
                BrowserKind::Chromium("Edge Dev".into()),
                Some("Dev".into()),
                format!("{}/Library/Application Support/Microsoft Edge Dev", home),
            ),
            (
                "Microsoft Edge Canary",
                BrowserKind::Chromium("Edge Canary".into()),
                Some("Canary".into()),
                format!("{}/Library/Application Support/Microsoft Edge Canary", home),
            ),
            (
                "Chromium",
                BrowserKind::Chromium("Chromium".into()),
                None,
                format!("{}/Library/Application Support/Chromium", home),
            ),
            (
                "Arc",
                BrowserKind::Chromium("Arc".into()),
                None,
                format!("{}/Library/Application Support/Arc/User Data", home),
            ),
            (
                "Vivaldi",
                BrowserKind::Chromium("Vivaldi".into()),
                None,
                format!("{}/Library/Application Support/Vivaldi", home),
            ),
            (
                "Firefox",
                BrowserKind::Firefox("Firefox".into()),
                None,
                format!("{}/Library/Application Support/Firefox", home),
            ),
            (
                "Firefox Developer Edition",
                BrowserKind::Firefox("Firefox Developer Edition".into()),
                Some("Developer Edition".into()),
                format!("{}/Library/Application Support/Firefox/Profiles", home),
            ),
            (
                "Firefox Nightly",
                BrowserKind::Firefox("Firefox Nightly".into()),
                Some("Nightly".into()),
                format!("{}/Library/Application Support/Firefox Nightly", home),
            ),
            (
                "Waterfox",
                BrowserKind::Firefox("Waterfox".into()),
                None,
                format!("{}/Library/Application Support/Waterfox", home),
            ),
            (
                "LibreWolf",
                BrowserKind::Firefox("LibreWolf".into()),
                None,
                format!("{}/Library/Application Support/LibreWolf", home),
            ),
            (
                "Floorp",
                BrowserKind::Firefox("Floorp".into()),
                None,
                format!("{}/Library/Application Support/Floorp", home),
            ),
            (
                "Zen Browser",
                BrowserKind::Firefox("Zen".into()),
                None,
                format!("{}/Library/Application Support/zen", home),
            ),
            (
                "Safari",
                BrowserKind::Safari,
                None,
                format!("{}/Library/Safari/Bookmarks.plist", home),
            ),
        ];

        let mut discovered = Vec::new();
        for (name, kind, channel, path_str) in candidates {
            let path = PathBuf::from(path_str);
            if path.exists() {
                discovered.push(BrowserInstallation {
                    name: name.to_string(),
                    kind,
                    channel,
                    root_path: path,
                });
            }
        }

        discovered
    }

    /// Discover profiles for a given installation
    pub fn discover_profiles(installation: &BrowserInstallation) -> Vec<BrowserProfile> {
        match &installation.kind {
            BrowserKind::Chromium(_) => Self::discover_chromium_profiles(installation),
            BrowserKind::Firefox(_) => Self::discover_firefox_profiles(installation),
            BrowserKind::Safari => {
                vec![BrowserProfile {
                    id: "default".into(),
                    name: "Safari Favorites & Bookmarks".into(),
                    path: installation.root_path.clone(),
                }]
            }
        }
    }

    fn discover_chromium_profiles(installation: &BrowserInstallation) -> Vec<BrowserProfile> {
        let mut profiles = Vec::new();
        let user_data_dir = &installation.root_path;

        // Parse Local State for display names if available
        let mut name_map = BTreeMap::new();
        let local_state_path = user_data_dir.join("Local State");
        if local_state_path.exists() {
            if let Ok(content) = fs::read_to_string(&local_state_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(info_cache) = json.get("profile").and_then(|p| p.get("info_cache"))
                    {
                        if let Some(obj) = info_cache.as_object() {
                            for (folder, info) in obj {
                                if let Some(name) = info.get("name").and_then(|n| n.as_str()) {
                                    name_map.insert(folder.clone(), name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Scan directories for Bookmarks file
        if let Ok(entries) = fs::read_dir(user_data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let folder_name = entry.file_name().to_string_lossy().to_string();
                    if folder_name == "Default" || folder_name.starts_with("Profile ") {
                        let bookmarks_file = path.join("Bookmarks");
                        if bookmarks_file.exists() {
                            let display_name = name_map
                                .get(&folder_name)
                                .cloned()
                                .unwrap_or_else(|| folder_name.clone());
                            profiles.push(BrowserProfile {
                                id: folder_name,
                                name: display_name,
                                path,
                            });
                        }
                    }
                }
            }
        }

        if profiles.is_empty() {
            let default_bm = user_data_dir.join("Default").join("Bookmarks");
            if default_bm.exists() {
                profiles.push(BrowserProfile {
                    id: "Default".into(),
                    name: "Default Profile".into(),
                    path: user_data_dir.join("Default"),
                });
            }
        }

        profiles
    }

    fn discover_firefox_profiles(installation: &BrowserInstallation) -> Vec<BrowserProfile> {
        let mut profiles = Vec::new();
        let profiles_dir = if installation.root_path.join("Profiles").exists() {
            installation.root_path.join("Profiles")
        } else {
            installation.root_path.clone()
        };

        if let Ok(entries) = fs::read_dir(&profiles_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let sqlite = path.join("places.sqlite");
                    if sqlite.exists() {
                        let id = entry.file_name().to_string_lossy().to_string();
                        let name = id.split('.').nth(1).unwrap_or(&id).to_string();
                        profiles.push(BrowserProfile {
                            id: id.clone(),
                            name,
                            path,
                        });
                    }
                }
            }
        }

        profiles
    }

    /// Harvest all bookmarks from discovered installations with snapshot isolation
    pub fn collect_all(output_dir: &Path) -> Result<(BookmarkTree, InventoryReport, RunManifest)> {
        let installations = Self::discover_installations();
        let snapshots_dir = output_dir.join("snapshots");
        fs::create_dir_all(&snapshots_dir)?;

        let mut sources = Vec::new();
        let mut tree_roots = Vec::new();

        let mut total_profiles_count = 0;
        let mut total_bookmarks_count = 0;

        let mut inst_summaries = Vec::new();
        let mut snapshot_hashes = BTreeMap::new();

        let mut warnings = Vec::new();

        for inst in &installations {
            let profiles = Self::discover_profiles(inst);
            total_profiles_count += profiles.len();

            let mut profile_summaries = Vec::new();

            if profiles.is_empty() {
                warnings.push(format!("Browser installation '{}' found at {:?} but has no active profiles with bookmarks.", inst.name, inst.root_path));
            }

            for prof in &profiles {
                let snapshot = create_profile_snapshot(prof, &snapshots_dir)?;
                for (k, v) in &snapshot.blake3_hashes {
                    snapshot_hashes.insert(format!("{}/{}", prof.id, k), v.clone());
                }

                let source_desc = SourceDescriptor {
                    browser: inst.kind.clone(),
                    browser_channel: inst.channel.clone(),
                    profile_id: prof.id.clone(),
                    profile_name: prof.name.clone(),
                    source_path: prof.path.clone(),
                };
                sources.push(source_desc.clone());

                let (prof_nodes, bm_count, folder_count) =
                    Self::harvest_snapshot_bookmarks(inst, prof, &snapshot, &source_desc)?;

                if bm_count == 0 {
                    warnings.push(format!("Profile '{}' (`{}`) in '{}' was scanned but contained 0 valid bookmark occurrences.", prof.name, prof.id, inst.name));
                }

                total_bookmarks_count += bm_count;

                // Root folder node wrapping this profile's harvest
                let root_folder = FolderNode {
                    id: format!("{}-{}", inst.name, prof.id),
                    title: format!("{} ({})", inst.name, prof.name),
                    children: prof_nodes,
                    source: source_desc,
                    root_kind: BookmarkRootKind::Custom("BrowserProfileRoot".into()),
                    original_position: tree_roots.len(),
                    date_added: Some(Utc::now()),
                    date_modified: Some(Utc::now()),
                };

                tree_roots.push(BookmarkNode::Folder(root_folder));

                profile_summaries.push(BrowserProfileSummary {
                    profile: prof.clone(),
                    bookmark_count: bm_count,
                    folder_count,
                    snapshot_path: Some(snapshot.snapshot_path),
                });
            }

            inst_summaries.push(BrowserInstallationSummary {
                installation: inst.clone(),
                profiles: profile_summaries,
            });
        }

        let tree = BookmarkTree {
            sources,
            roots: tree_roots,
            harvested_at: Utc::now(),
        };

        let inventory = InventoryReport {
            schema_version: "1.0.0".into(),
            generated_at: Utc::now(),
            total_installations: installations.len(),
            total_profiles: total_profiles_count,
            total_bookmarks: total_bookmarks_count,
            warnings: warnings.clone(),
            installations: inst_summaries,
        };

        let manifest = RunManifest {
            schema_version: "1.0.0".into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            options: BTreeMap::new(),
            total_sources: inventory.total_profiles,
            total_raw_occurrences: total_bookmarks_count,
            total_canonical_bookmarks: 0,
            total_removed_duplicates: 0,
            invariant_satisfied: true,
            warnings,
            output_files: BTreeMap::new(),
            blake3_snapshot_hashes: snapshot_hashes,
        };

        Ok((tree, inventory, manifest))
    }

    fn harvest_snapshot_bookmarks(
        inst: &BrowserInstallation,
        _prof: &BrowserProfile,
        snapshot: &ProfileSnapshot,
        source: &SourceDescriptor,
    ) -> Result<(Vec<BookmarkNode>, usize, usize)> {
        match &inst.kind {
            BrowserKind::Chromium(_) => {
                let bm_file = snapshot.snapshot_path.join("Bookmarks");
                if bm_file.exists() {
                    let content = fs::read_to_string(&bm_file)?;
                    let json: serde_json::Value = serde_json::from_str(&content)?;
                    Self::parse_chromium_json(&json, source)
                } else {
                    Ok((vec![], 0, 0))
                }
            }
            BrowserKind::Firefox(_) => {
                let db_file = snapshot.snapshot_path.join("places.sqlite");
                if db_file.exists() {
                    Self::parse_firefox_sqlite(&db_file, source)
                } else {
                    Ok((vec![], 0, 0))
                }
            }
            BrowserKind::Safari => {
                let plist_file = snapshot.snapshot_path.join("Bookmarks.plist");
                if plist_file.exists() {
                    Self::parse_safari_plist(&plist_file, source)
                } else {
                    Ok((vec![], 0, 0))
                }
            }
        }
    }

    fn parse_chromium_json(
        json: &serde_json::Value,
        source: &SourceDescriptor,
    ) -> Result<(Vec<BookmarkNode>, usize, usize)> {
        let mut nodes = Vec::new();
        let mut bm_count = 0;
        let mut folder_count = 0;

        if let Some(roots) = json.get("roots").and_then(|r| r.as_object()) {
            for (root_key, root_val) in roots {
                let root_kind = match root_key.as_str() {
                    "bookmark_bar" => BookmarkRootKind::BookmarkBar,
                    "other" => BookmarkRootKind::OtherBookmarks,
                    "synced" | "mobile" => BookmarkRootKind::MobileBookmarks,
                    other => BookmarkRootKind::Custom(other.to_string()),
                };

                let (children, bm, fld) = Self::parse_chromium_children(
                    root_val.get("children"),
                    source,
                    &root_kind,
                    &[root_val
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or(root_key)
                        .to_string()],
                )?;

                bm_count += bm;
                folder_count += fld + 1;

                let title = root_val
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(root_key)
                    .to_string();

                let folder_node = FolderNode {
                    id: format!("{}-root-{}", source.profile_id, root_key),
                    title,
                    children,
                    source: source.clone(),
                    root_kind,
                    original_position: nodes.len(),
                    date_added: None,
                    date_modified: None,
                };
                nodes.push(BookmarkNode::Folder(folder_node));
            }
        }

        Ok((nodes, bm_count, folder_count))
    }

    fn parse_chromium_children(
        children_val: Option<&serde_json::Value>,
        source: &SourceDescriptor,
        root_kind: &BookmarkRootKind,
        parent_path: &[String],
    ) -> Result<(Vec<BookmarkNode>, usize, usize)> {
        let mut nodes = Vec::new();
        let mut bm_count = 0;
        let mut folder_count = 0;

        if let Some(arr) = children_val.and_then(|c| c.as_array()) {
            for (idx, item) in arr.iter().enumerate() {
                let node_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("url");
                let title = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let node_id = item
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string();

                if node_type == "folder" {
                    folder_count += 1;
                    let mut child_path = parent_path.to_vec();
                    child_path.push(title.clone());

                    let (children, child_bm, child_fld) = Self::parse_chromium_children(
                        item.get("children"),
                        source,
                        root_kind,
                        &child_path,
                    )?;
                    bm_count += child_bm;
                    folder_count += child_fld;

                    nodes.push(BookmarkNode::Folder(FolderNode {
                        id: node_id,
                        title,
                        children,
                        source: source.clone(),
                        root_kind: root_kind.clone(),
                        original_position: idx,
                        date_added: None,
                        date_modified: None,
                    }));
                } else if node_type == "url" {
                    if let Some(url) = item.get("url").and_then(|u| u.as_str()) {
                        bm_count += 1;
                        nodes.push(BookmarkNode::Bookmark(BookmarkOccurrence {
                            id: node_id,
                            title,
                            raw_url: url.to_string(),
                            source: source.clone(),
                            root_kind: root_kind.clone(),
                            folder_path: parent_path.to_vec(),
                            original_position: idx,
                            date_added: None,
                            date_modified: None,
                            metadata: BTreeMap::new(),
                        }));
                    }
                }
            }
        }

        Ok((nodes, bm_count, folder_count))
    }

    fn parse_firefox_sqlite(
        db_path: &Path,
        source: &SourceDescriptor,
    ) -> Result<(Vec<BookmarkNode>, usize, usize)> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;

        let mut stmt = conn.prepare(
            "SELECT b.id, b.type, b.title, p.url, b.dateAdded, b.lastModified, b.parent
             FROM moz_bookmarks b
             LEFT JOIN moz_places p ON b.fk = p.id
             WHERE b.type IN (1, 2, 3)",
        )?;

        let mut bm_count = 0;
        let mut folder_count = 0;
        let mut nodes = Vec::new();

        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let btype: i64 = row.get(1)?;
            let title: Option<String> = row.get(2)?;
            let url: Option<String> = row.get(3)?;
            let date_added: Option<i64> = row.get(4)?;
            let date_modified: Option<i64> = row.get(5)?;
            Ok((id, btype, title, url, date_added, date_modified))
        })?;

        for (idx, r) in rows.flatten().enumerate() {
            let (id, btype, title_opt, url_opt, d_add, d_mod) = r;
            let title = title_opt.unwrap_or_default();

            if btype == 1 {
                // URL Bookmark
                if let Some(raw_url) = url_opt {
                    bm_count += 1;
                    nodes.push(BookmarkNode::Bookmark(BookmarkOccurrence {
                        id: id.to_string(),
                        title,
                        raw_url,
                        source: source.clone(),
                        root_kind: BookmarkRootKind::BookmarkMenu,
                        folder_path: vec!["Firefox Places".into()],
                        original_position: idx,
                        date_added: d_add
                            .map(|t| DateTime::from_timestamp_micros(t).unwrap_or_default()),
                        date_modified: d_mod
                            .map(|t| DateTime::from_timestamp_micros(t).unwrap_or_default()),
                        metadata: BTreeMap::new(),
                    }));
                }
            } else if btype == 2 {
                // Folder
                folder_count += 1;
                nodes.push(BookmarkNode::Folder(FolderNode {
                    id: id.to_string(),
                    title,
                    children: vec![],
                    source: source.clone(),
                    root_kind: BookmarkRootKind::BookmarkMenu,
                    original_position: idx,
                    date_added: d_add
                        .map(|t| DateTime::from_timestamp_micros(t).unwrap_or_default()),
                    date_modified: d_mod
                        .map(|t| DateTime::from_timestamp_micros(t).unwrap_or_default()),
                }));
            } else if btype == 3 {
                // Separator
                nodes.push(BookmarkNode::Separator(SeparatorNode {
                    id: id.to_string(),
                    source: source.clone(),
                    original_position: idx,
                }));
            }
        }

        Ok((nodes, bm_count, folder_count))
    }

    fn parse_safari_plist(
        plist_path: &Path,
        source: &SourceDescriptor,
    ) -> Result<(Vec<BookmarkNode>, usize, usize)> {
        let data = fs::read(plist_path)?;
        let plist_value: plist::Value = plist::from_bytes(&data)?;

        let mut nodes = Vec::new();
        let mut bm_count = 0;
        let mut folder_count = 0;

        if let Some(dict) = plist_value.as_dictionary() {
            if let Some(children) = dict.get("Children").and_then(|v| v.as_array()) {
                for (idx, child) in children.iter().enumerate() {
                    let (child_nodes, bm, fld) = Self::parse_safari_node(
                        child,
                        source,
                        &BookmarkRootKind::Favorites,
                        &["Favorites".into()],
                        idx,
                    )?;
                    bm_count += bm;
                    folder_count += fld;
                    nodes.extend(child_nodes);
                }
            }
        }

        Ok((nodes, bm_count, folder_count))
    }

    fn parse_safari_node(
        node: &plist::Value,
        source: &SourceDescriptor,
        root_kind: &BookmarkRootKind,
        parent_path: &[String],
        idx: usize,
    ) -> Result<(Vec<BookmarkNode>, usize, usize)> {
        let mut nodes = Vec::new();
        let mut bm_count = 0;
        let mut folder_count = 0;

        let dict = match node.as_dictionary() {
            Some(d) => d,
            None => return Ok((nodes, bm_count, folder_count)),
        };

        let web_bookmark_type = dict
            .get("WebBookmarkType")
            .and_then(|v| v.as_string())
            .unwrap_or("");

        let title = dict
            .get("Title")
            .and_then(|v| v.as_string())
            .or_else(|| {
                dict.get("URIDictionary")
                    .and_then(|v| v.as_dictionary())
                    .and_then(|d| d.get("title"))
                    .and_then(|v| v.as_string())
            })
            .unwrap_or("")
            .to_string();

        let uuid = dict
            .get("WebBookmarkUUID")
            .and_then(|v| v.as_string())
            .unwrap_or("")
            .to_string();

        match web_bookmark_type {
            "WebBookmarkTypeList" => {
                if title == "com.apple.ReadingList" || title == "History" {
                    return Ok((nodes, bm_count, folder_count));
                }

                folder_count += 1;
                let mut child_path = parent_path.to_vec();
                child_path.push(title.clone());

                let mut sub_nodes = Vec::new();
                if let Some(child_array) = dict.get("Children").and_then(|v| v.as_array()) {
                    for (c_idx, child) in child_array.iter().enumerate() {
                        let (c_nodes, c_bm, c_fld) =
                            Self::parse_safari_node(child, source, root_kind, &child_path, c_idx)?;
                        bm_count += c_bm;
                        folder_count += c_fld;
                        sub_nodes.extend(c_nodes);
                    }
                }

                nodes.push(BookmarkNode::Folder(FolderNode {
                    id: uuid,
                    title,
                    children: sub_nodes,
                    source: source.clone(),
                    root_kind: root_kind.clone(),
                    original_position: idx,
                    date_added: None,
                    date_modified: None,
                }));
            }
            "WebBookmarkTypeLeaf" => {
                let url = dict
                    .get("URLString")
                    .and_then(|v| v.as_string())
                    .unwrap_or("");

                if !url.is_empty() {
                    bm_count += 1;
                    nodes.push(BookmarkNode::Bookmark(BookmarkOccurrence {
                        id: uuid,
                        title,
                        raw_url: url.to_string(),
                        source: source.clone(),
                        root_kind: root_kind.clone(),
                        folder_path: parent_path.to_vec(),
                        original_position: idx,
                        date_added: None,
                        date_modified: None,
                        metadata: BTreeMap::new(),
                    }));
                }
            }
            _ => {}
        }

        Ok((nodes, bm_count, folder_count))
    }
}
