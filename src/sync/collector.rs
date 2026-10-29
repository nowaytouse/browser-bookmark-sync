//! Comprehensive Browser and Profile Harvester with Snapshot Isolation

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashSet};
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
    pub fn discover_installations() -> Result<Vec<BrowserInstallation>> {
        let (installations, issues) = Self::discover_installations_with_report()?;
        for issue in issues {
            tracing::warn!("Browser discovery incomplete: {}", issue["error"]);
        }
        Ok(installations)
    }

    pub fn discover_installations_with_report(
    ) -> Result<(Vec<BrowserInstallation>, Vec<serde_json::Value>)> {
        Self::discover_installations_cancellable(&super::cancellation::Cancellation::default())
    }

    pub(crate) fn discover_installations_cancellable(
        cancellation: &super::cancellation::Cancellation,
    ) -> Result<(Vec<BrowserInstallation>, Vec<serde_json::Value>)> {
        Self::discover_installations_from_home_cancellable(
            std::env::var_os("HOME").map(PathBuf::from),
            cancellation,
        )
    }

    #[cfg(test)]
    fn discover_installations_from_home(
        home: Option<PathBuf>,
    ) -> Result<(Vec<BrowserInstallation>, Vec<serde_json::Value>)> {
        Self::discover_installations_from_home_cancellable(
            home,
            &super::cancellation::Cancellation::default(),
        )
    }

    fn discover_installations_from_home_cancellable(
        home: Option<PathBuf>,
        cancellation: &super::cancellation::Cancellation,
    ) -> Result<(Vec<BrowserInstallation>, Vec<serde_json::Value>)> {
        cancellation.check()?;
        let home = home.context("HOME is required for full browser profile discovery")?;

        let candidates = vec![
            (
                "Google Chrome",
                BrowserKind::Chromium("Chrome".into()),
                None,
                home.join("Library/Application Support/Google/Chrome"),
            ),
            (
                "Google Chrome Beta",
                BrowserKind::Chromium("Chrome Beta".into()),
                Some("Beta".into()),
                home.join("Library/Application Support/Google/Chrome Beta"),
            ),
            (
                "Google Chrome Dev",
                BrowserKind::Chromium("Chrome Dev".into()),
                Some("Dev".into()),
                home.join("Library/Application Support/Google/Chrome Dev"),
            ),
            (
                "Google Chrome Canary",
                BrowserKind::Chromium("Chrome Canary".into()),
                Some("Canary".into()),
                home.join("Library/Application Support/Google/Chrome Canary"),
            ),
            (
                "Brave",
                BrowserKind::Chromium("Brave".into()),
                None,
                home.join("Library/Application Support/BraveSoftware/Brave-Browser"),
            ),
            (
                "Brave Beta",
                BrowserKind::Chromium("Brave Beta".into()),
                Some("Beta".into()),
                home.join("Library/Application Support/BraveSoftware/Brave-Browser-Beta"),
            ),
            (
                "Brave Nightly",
                BrowserKind::Chromium("Brave Nightly".into()),
                Some("Nightly".into()),
                home.join("Library/Application Support/BraveSoftware/Brave-Browser-Nightly"),
            ),
            (
                "Microsoft Edge",
                BrowserKind::Chromium("Edge".into()),
                None,
                home.join("Library/Application Support/Microsoft Edge"),
            ),
            (
                "Microsoft Edge Beta",
                BrowserKind::Chromium("Edge Beta".into()),
                Some("Beta".into()),
                home.join("Library/Application Support/Microsoft Edge Beta"),
            ),
            (
                "Microsoft Edge Dev",
                BrowserKind::Chromium("Edge Dev".into()),
                Some("Dev".into()),
                home.join("Library/Application Support/Microsoft Edge Dev"),
            ),
            (
                "Microsoft Edge Canary",
                BrowserKind::Chromium("Edge Canary".into()),
                Some("Canary".into()),
                home.join("Library/Application Support/Microsoft Edge Canary"),
            ),
            (
                "Chromium",
                BrowserKind::Chromium("Chromium".into()),
                None,
                home.join("Library/Application Support/Chromium"),
            ),
            (
                "Arc",
                BrowserKind::Chromium("Arc".into()),
                None,
                home.join("Library/Application Support/Arc/User Data"),
            ),
            (
                "Vivaldi",
                BrowserKind::Chromium("Vivaldi".into()),
                None,
                home.join("Library/Application Support/Vivaldi"),
            ),
            (
                "Firefox",
                BrowserKind::Firefox("Firefox".into()),
                None,
                home.join("Library/Application Support/Firefox"),
            ),
            (
                "Firefox Nightly",
                BrowserKind::Firefox("Firefox Nightly".into()),
                Some("Nightly".into()),
                home.join("Library/Application Support/Firefox Nightly"),
            ),
            (
                "Waterfox",
                BrowserKind::Firefox("Waterfox".into()),
                None,
                home.join("Library/Application Support/Waterfox"),
            ),
            (
                "LibreWolf",
                BrowserKind::Firefox("LibreWolf".into()),
                None,
                home.join("Library/Application Support/LibreWolf"),
            ),
            (
                "Floorp",
                BrowserKind::Firefox("Floorp".into()),
                None,
                home.join("Library/Application Support/Floorp"),
            ),
            (
                "Zen Browser",
                BrowserKind::Firefox("Zen".into()),
                None,
                home.join("Library/Application Support/zen"),
            ),
            (
                "Safari",
                BrowserKind::Safari,
                None,
                home.join("Library/Safari/Bookmarks.plist"),
            ),
            (
                "Safari Technology Preview",
                BrowserKind::Safari,
                Some("Technology Preview".into()),
                home.join(
                    "Library/Containers/com.apple.SafariTechnologyPreview/Data/Library/Safari/Bookmarks.plist",
                ),
            ),
        ];

        let mut discovered = Vec::new();
        let mut issues = Vec::new();
        for (name, kind, channel, path) in candidates {
            cancellation.check()?;
            match path.try_exists() {
                Ok(true) => discovered.push(BrowserInstallation {
                    name: name.to_string(),
                    kind,
                    channel,
                    root_path: path,
                }),
                Ok(false) => {}
                Err(error) => issues.push(Self::discovery_issue(&path, Some(name), &error.into())),
            }
        }

        // Pick up Chromium/Gecko forks not present in the static name list. Search only
        // application-support roots and one vendor nesting level to keep discovery bounded.
        let app_support = home.join("Library/Application Support");
        let entries = fs::read_dir(&app_support)
            .with_context(|| format!("Cannot read Application Support at {app_support:?}"))?;
        for entry in entries {
            cancellation.check()?;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    issues.push(Self::discovery_issue(&app_support, None, &error.into()));
                    continue;
                }
            };
            let top = entry.path();
            let is_dir = match entry.file_type() {
                Ok(kind) => kind.is_dir(),
                Err(error) => {
                    issues.push(Self::discovery_issue(&top, None, &error.into()));
                    continue;
                }
            };
            if !is_dir || discovered.iter().any(|item| item.root_path == top) {
                continue;
            }
            let probe_failed = match Self::browser_at_root(&top) {
                Ok(Some(installation)) => {
                    discovered.push(installation);
                    continue;
                }
                Ok(None) => false,
                Err(error) => {
                    issues.push(Self::discovery_issue(&top, None, &error));
                    true
                }
            };
            let children = match fs::read_dir(&top) {
                Ok(children) => children,
                Err(error) => {
                    if !probe_failed {
                        issues.push(Self::discovery_issue(&top, None, &error.into()));
                    }
                    continue;
                }
            };
            for child in children {
                cancellation.check()?;
                let child = match child {
                    Ok(child) => child,
                    Err(error) => {
                        issues.push(Self::discovery_issue(&top, None, &error.into()));
                        continue;
                    }
                };
                let path = child.path();
                let is_dir = match child.file_type() {
                    Ok(kind) => kind.is_dir(),
                    Err(error) => {
                        issues.push(Self::discovery_issue(&path, None, &error.into()));
                        continue;
                    }
                };
                if !is_dir || discovered.iter().any(|item| item.root_path == path) {
                    continue;
                }
                match Self::browser_at_root(&path) {
                    Ok(Some(installation)) => discovered.push(installation),
                    Ok(None) => {}
                    Err(error) => issues.push(Self::discovery_issue(&path, None, &error)),
                }
            }
        }

        Self::deduplicate_installations(&mut discovered);
        discovered.sort_by(|left, right| left.name.cmp(&right.name));
        cancellation.check()?;
        Ok((discovered, issues))
    }

    fn browser_at_root(path: &Path) -> Result<Option<BrowserInstallation>> {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let kind = if Self::looks_like_firefox_root(path)? {
            BrowserKind::Firefox(name.clone())
        } else if Self::looks_like_chromium_root(path)? {
            BrowserKind::Chromium(name.clone())
        } else {
            return Ok(None);
        };
        Ok(Some(BrowserInstallation {
            name,
            kind,
            channel: None,
            root_path: path.to_path_buf(),
        }))
    }

    fn discovery_issue(
        path: &Path,
        browser: Option<&str>,
        error: &anyhow::Error,
    ) -> serde_json::Value {
        serde_json::json!({
            "source_type": "browser_discovery",
            "browser": browser.unwrap_or("Unclassified directory"),
            "profile": "",
            "path": path,
            "status": "unverified",
            "browser_identity_verified": false,
            "error": format!("Cannot inspect discovery candidate {path:?}: {error:#}"),
        })
    }

    fn installation_identity(path: &Path) -> String {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = fs::metadata(&canonical) {
                return format!("inode:{}:{}", metadata.dev(), metadata.ino());
            }
        }
        format!("path:{}", canonical.to_string_lossy())
    }

    fn deduplicate_installations(installations: &mut Vec<BrowserInstallation>) {
        let mut seen = HashSet::new();
        installations.retain(|installation| {
            seen.insert(Self::installation_identity(&installation.root_path))
        });
    }

    fn is_file_if_present(path: &Path) -> Result<bool> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).with_context(|| format!("Cannot inspect file {path:?}")),
        }
    }

    fn has_chromium_store(path: &Path, include_reading_lists: bool) -> Result<bool> {
        Ok(Self::is_file_if_present(&path.join("Bookmarks"))?
            || (include_reading_lists
                && Self::is_file_if_present(&path.join("Sync Data/LevelDB/CURRENT"))?))
    }

    fn looks_like_chromium_root(path: &Path) -> Result<bool> {
        if Self::has_chromium_store(path, true)? {
            return Ok(true);
        }
        let entries = fs::read_dir(path)
            .with_context(|| format!("Cannot inspect possible Chromium root {path:?}"))?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!("Cannot read an entry in possible Chromium root {path:?}")
            })?;
            let entry_path = entry.path();
            if entry
                .file_type()
                .with_context(|| {
                    format!("Cannot inspect possible Chromium profile {entry_path:?}")
                })?
                .is_dir()
                && Self::has_chromium_store(&entry_path, true)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn looks_like_firefox_root(path: &Path) -> Result<bool> {
        if Self::is_file_if_present(&path.join("profiles.ini"))? {
            return Ok(true);
        }
        let profiles_dir = path.join("Profiles");
        if !profiles_dir.try_exists().with_context(|| {
            format!("Cannot inspect possible Firefox profiles at {profiles_dir:?}")
        })? {
            return Ok(false);
        }
        let entries = fs::read_dir(&profiles_dir)
            .with_context(|| format!("Cannot inspect possible Firefox root {path:?}"))?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!("Cannot read an entry in possible Firefox root {path:?}")
            })?;
            let entry_path = entry.path();
            if entry
                .file_type()
                .with_context(|| format!("Cannot inspect possible Firefox profile {entry_path:?}"))?
                .is_dir()
                && Self::is_file_if_present(&entry_path.join("places.sqlite"))?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Discover profiles for a given installation
    pub fn discover_profiles(installation: &BrowserInstallation) -> Result<Vec<BrowserProfile>> {
        match &installation.kind {
            BrowserKind::Chromium(_) => Self::discover_chromium_profiles(installation, false),
            BrowserKind::Firefox(_) => Self::discover_firefox_profiles(installation),
            BrowserKind::Safari => Ok(vec![BrowserProfile {
                id: "default".into(),
                name: "Safari Favorites & Bookmarks".into(),
                path: installation.root_path.clone(),
            }]),
        }
    }

    pub(crate) fn discover_reading_list_profiles(
        installation: &BrowserInstallation,
    ) -> Result<Vec<BrowserProfile>> {
        match installation.kind {
            BrowserKind::Chromium(_) => Self::discover_chromium_profiles(installation, true),
            _ => Self::discover_profiles(installation),
        }
    }

    fn discover_chromium_profiles(
        installation: &BrowserInstallation,
        include_reading_lists: bool,
    ) -> Result<Vec<BrowserProfile>> {
        let mut profiles = Vec::new();
        let user_data_dir = &installation.root_path;
        let has_store = |path: &Path| Self::has_chromium_store(path, include_reading_lists);

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

        // Some Chromium browsers (notably single-profile layouts) store Bookmarks
        // directly in their application-support root.
        if has_store(user_data_dir)? {
            profiles.push(BrowserProfile {
                id: "Default".into(),
                name: name_map
                    .get("Default")
                    .cloned()
                    .unwrap_or_else(|| "Default Profile".into()),
                path: user_data_dir.clone(),
            });
        }

        // Scan directories for Bookmarks file
        let entries = fs::read_dir(user_data_dir).with_context(|| {
            format!(
                "Cannot read Chromium installation '{}' at {:?}",
                installation.name, user_data_dir
            )
        })?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    "Cannot read a profile entry in Chromium installation '{}' at {:?}",
                    installation.name, user_data_dir
                )
            })?;
            let path = entry.path();
            if entry
                .file_type()
                .with_context(|| format!("Cannot inspect Chromium profile candidate {path:?}"))?
                .is_dir()
            {
                let folder_name = entry.file_name().to_string_lossy().to_string();
                if has_store(&path)? {
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

        if profiles.is_empty() {
            let default_bm = user_data_dir.join("Default").join("Bookmarks");
            if Self::is_file_if_present(&default_bm)? {
                profiles.push(BrowserProfile {
                    id: "Default".into(),
                    name: "Default Profile".into(),
                    path: user_data_dir.join("Default"),
                });
            }
        }

        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(profiles)
    }

    fn discover_firefox_profiles(
        installation: &BrowserInstallation,
    ) -> Result<Vec<BrowserProfile>> {
        let mut profiles = BTreeMap::new();
        let conventional_profiles = installation.root_path.join("Profiles");
        let profiles_dir = if conventional_profiles.try_exists().with_context(|| {
            format!(
                "Cannot inspect Firefox Profiles directory for '{}'",
                installation.name
            )
        })? {
            conventional_profiles
        } else {
            installation.root_path.clone()
        };

        let entries = fs::read_dir(&profiles_dir).with_context(|| {
            format!(
                "Cannot read Firefox installation '{}' at {:?}",
                installation.name, profiles_dir
            )
        })?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    "Cannot read a profile entry in Firefox installation '{}' at {:?}",
                    installation.name, profiles_dir
                )
            })?;
            let path = entry.path();
            if entry
                .file_type()
                .with_context(|| format!("Cannot inspect Firefox profile candidate {path:?}"))?
                .is_dir()
            {
                let sqlite = path.join("places.sqlite");
                if Self::is_file_if_present(&sqlite)? {
                    let id = entry.file_name().to_string_lossy().to_string();
                    let name = id.split('.').nth(1).unwrap_or(&id).to_string();
                    profiles
                        .entry(path.clone())
                        .or_insert(BrowserProfile { id, name, path });
                }
            }
        }

        // profiles.ini is authoritative and may point outside the conventional
        // Profiles directory via IsRelative=0.
        let profiles_ini = installation.root_path.join("profiles.ini");
        if Self::is_file_if_present(&profiles_ini)? {
            let contents = fs::read_to_string(&profiles_ini).with_context(|| {
                format!("Cannot read Firefox profile registry {profiles_ini:?}")
            })?;
            let mut sections = Vec::new();
            let mut current: Option<BTreeMap<String, String>> = None;
            for line in contents.lines().map(str::trim) {
                if line.starts_with('[') && line.ends_with(']') {
                    if let Some(section) = current.take() {
                        sections.push(section);
                    }
                    current = line[1..line.len() - 1]
                        .starts_with("Profile")
                        .then(BTreeMap::new);
                } else if let Some(section) = current.as_mut() {
                    if let Some((key, value)) = line.split_once('=') {
                        section.insert(key.trim().to_string(), value.trim().to_string());
                    }
                }
            }
            if let Some(section) = current {
                sections.push(section);
            }

            for section in sections {
                let Some(configured_path) = section.get("Path") else {
                    continue;
                };
                let path = if section.get("IsRelative").is_some_and(|value| value == "0") {
                    PathBuf::from(configured_path)
                } else {
                    installation.root_path.join(configured_path)
                };
                if !Self::is_file_if_present(&path.join("places.sqlite"))? {
                    continue;
                }
                let id = configured_path.clone();
                let name = section.get("Name").cloned().unwrap_or_else(|| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
                profiles
                    .entry(path.clone())
                    .or_insert(BrowserProfile { id, name, path });
            }
        }

        Ok(profiles.into_values().collect())
    }

    fn remove_onion_nodes(nodes: &mut Vec<BookmarkNode>) -> usize {
        let mut removed = 0;
        nodes.retain_mut(|node| match node {
            BookmarkNode::Folder(folder) => {
                removed += Self::remove_onion_nodes(&mut folder.children);
                true
            }
            BookmarkNode::Bookmark(bookmark) if super::is_onion_url(&bookmark.raw_url) => {
                removed += 1;
                false
            }
            BookmarkNode::Bookmark(_) | BookmarkNode::Separator(_) => true,
        });
        removed
    }

    /// Harvest all bookmarks from discovered installations with snapshot isolation
    pub fn collect_all(output_dir: &Path) -> Result<(BookmarkTree, InventoryReport, RunManifest)> {
        Self::collect_all_with_options(output_dir, true)
    }

    /// Harvest every discovered profile, optionally excluding onion URLs before aggregation.
    pub fn collect_all_with_options(
        output_dir: &Path,
        exclude_onion: bool,
    ) -> Result<(BookmarkTree, InventoryReport, RunManifest)> {
        Self::collect_all_cancellable(
            output_dir,
            exclude_onion,
            &super::cancellation::Cancellation::default(),
        )
    }

    pub(crate) fn collect_all_cancellable(
        output_dir: &Path,
        exclude_onion: bool,
        cancellation: &super::cancellation::Cancellation,
    ) -> Result<(BookmarkTree, InventoryReport, RunManifest)> {
        cancellation.check()?;
        let (installations, issues) = Self::discover_installations_cancellable(cancellation)?;
        let (tree, mut inventory, mut manifest) =
            Self::collect_discovered_installations_cancellable(
                output_dir,
                exclude_onion,
                installations,
                cancellation,
            )?;
        for issue in issues {
            if let Some(error) = issue["error"].as_str() {
                inventory.warnings.push(error.to_owned());
                manifest.warnings.push(error.to_owned());
            }
        }
        cancellation.check()?;
        Ok((tree, inventory, manifest))
    }

    #[cfg(test)]
    fn collect_discovered_installations(
        output_dir: &Path,
        exclude_onion: bool,
        installations: Vec<BrowserInstallation>,
    ) -> Result<(BookmarkTree, InventoryReport, RunManifest)> {
        Self::collect_discovered_installations_cancellable(
            output_dir,
            exclude_onion,
            installations,
            &super::cancellation::Cancellation::default(),
        )
    }

    fn collect_discovered_installations_cancellable(
        output_dir: &Path,
        exclude_onion: bool,
        installations: Vec<BrowserInstallation>,
        cancellation: &super::cancellation::Cancellation,
    ) -> Result<(BookmarkTree, InventoryReport, RunManifest)> {
        cancellation.check()?;
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
            cancellation.check()?;
            let profiles_result = Self::discover_profiles(inst);
            cancellation.check()?;
            let profiles = match profiles_result {
                Ok(profiles) => profiles,
                Err(error) => {
                    let message = format!(
                        "Cannot complete profile discovery for '{}' at {:?}: {error:#}",
                        inst.name, inst.root_path
                    );
                    warnings.push(message.clone());
                    inst_summaries.push(BrowserInstallationSummary {
                        installation: inst.clone(),
                        profiles: Vec::new(),
                        discovery_error: Some(message),
                    });
                    continue;
                }
            };
            total_profiles_count += profiles.len();

            let mut profile_summaries = Vec::new();

            if profiles.is_empty() {
                warnings.push(format!("Browser installation '{}' found at {:?} but has no active profiles with bookmarks.", inst.name, inst.root_path));
            }

            for prof in &profiles {
                cancellation.check()?;
                let snapshot_root = snapshots_dir.join(inst.name.replace(['/', ':'], "_"));
                let source_desc = SourceDescriptor {
                    browser: inst.kind.clone(),
                    browser_channel: inst.channel.clone(),
                    profile_id: prof.id.clone(),
                    profile_name: prof.name.clone(),
                    source_path: prof.path.clone(),
                };
                sources.push(source_desc.clone());

                let mut prof_nodes = Vec::new();
                let mut bm_count = 0;
                let mut folder_count = 0;
                let mut snapshot_path = None;
                let mut read_success = true;
                let mut read_error = None;

                let snapshot_result = create_profile_snapshot(prof, &snapshot_root);
                cancellation.check()?;
                match snapshot_result {
                    Ok(snapshot) => {
                        snapshot_path = Some(snapshot.snapshot_path.clone());
                        for (k, v) in &snapshot.blake3_hashes {
                            snapshot_hashes
                                .insert(format!("{}/{}/{}", inst.name, prof.id, k), v.clone());
                        }
                        cancellation.check()?;
                        let harvest_result =
                            Self::harvest_snapshot_bookmarks(inst, prof, &snapshot, &source_desc);
                        cancellation.check()?;
                        match harvest_result {
                            Ok((nodes, count, folders)) => {
                                prof_nodes = nodes;
                                bm_count = count;
                                folder_count = folders;
                            }
                            Err(error) => {
                                read_success = false;
                                read_error = Some(format!(
                                    "failed to harvest profile '{}' in '{}': {error:#}",
                                    prof.name, inst.name
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        read_success = false;
                        read_error = Some(format!(
                            "failed to snapshot profile '{}' in '{}': {error:#}",
                            prof.name, inst.name
                        ));
                    }
                }

                if let Some(error) = read_error.as_ref() {
                    warnings.push(error.clone());
                }

                if exclude_onion {
                    bm_count = bm_count.saturating_sub(Self::remove_onion_nodes(&mut prof_nodes));
                }

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
                    snapshot_path,
                    read_success,
                    error: read_error,
                });
            }

            inst_summaries.push(BrowserInstallationSummary {
                installation: inst.clone(),
                profiles: profile_summaries,
                discovery_error: None,
            });
        }

        let has_safari_source = sources
            .iter()
            .any(|source| matches!(&source.browser, BrowserKind::Safari));
        cancellation.check()?;
        anyhow::ensure!(
            total_profiles_count > 0 || has_safari_source,
            "Full bookmark collection found no readable browser profiles and no Safari source"
        );

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
            options: BTreeMap::from([
                ("collection_scope".into(), "all_profiles".into()),
                ("exclude_onion".into(), exclude_onion.to_string()),
            ]),
            total_sources: inventory.total_profiles,
            total_raw_occurrences: total_bookmarks_count,
            total_canonical_bookmarks: 0,
            total_removed_duplicates: 0,
            invariant_satisfied: true,
            warnings,
            output_files: BTreeMap::new(),
            blake3_snapshot_hashes: snapshot_hashes,
        };

        cancellation.check()?;
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
        #[derive(Clone)]
        struct FirefoxRow {
            id: i64,
            kind: i64,
            title: String,
            url: Option<String>,
            date_added: Option<i64>,
            date_modified: Option<i64>,
            parent: i64,
            position: usize,
        }

        fn parse_time(value: i64) -> DateTime<Utc> {
            DateTime::from_timestamp_micros(value).unwrap_or_else(|| {
                tracing::warn!("Firefox Places timestamp {value} is out of range");
                DateTime::UNIX_EPOCH
            })
        }

        fn build_node(
            id: i64,
            rows: &mut BTreeMap<i64, FirefoxRow>,
            children: &BTreeMap<i64, Vec<i64>>,
            source: &SourceDescriptor,
            root_kind: &BookmarkRootKind,
            parent_path: &[String],
            bookmark_count: &mut usize,
            folder_count: &mut usize,
        ) -> Option<BookmarkNode> {
            let row = rows.remove(&id)?;
            match row.kind {
                1 => {
                    let raw_url = row.url?;
                    *bookmark_count += 1;
                    Some(BookmarkNode::Bookmark(BookmarkOccurrence {
                        id: row.id.to_string(),
                        title: row.title,
                        raw_url,
                        source: source.clone(),
                        root_kind: root_kind.clone(),
                        folder_path: parent_path.to_vec(),
                        original_position: row.position,
                        date_added: row.date_added.map(parse_time),
                        date_modified: row.date_modified.map(parse_time),
                        metadata: BTreeMap::new(),
                    }))
                }
                2 => {
                    *folder_count += 1;
                    let mut child_path = parent_path.to_vec();
                    child_path.push(row.title.clone());
                    let nested = children
                        .get(&row.id)
                        .into_iter()
                        .flatten()
                        .filter_map(|child_id| {
                            build_node(
                                *child_id,
                                rows,
                                children,
                                source,
                                root_kind,
                                &child_path,
                                bookmark_count,
                                folder_count,
                            )
                        })
                        .collect();
                    Some(BookmarkNode::Folder(FolderNode {
                        id: row.id.to_string(),
                        title: row.title,
                        children: nested,
                        source: source.clone(),
                        root_kind: root_kind.clone(),
                        original_position: row.position,
                        date_added: row.date_added.map(parse_time),
                        date_modified: row.date_modified.map(parse_time),
                    }))
                }
                3 => Some(BookmarkNode::Separator(SeparatorNode {
                    id: row.id.to_string(),
                    source: source.clone(),
                    original_position: row.position,
                })),
                _ => None,
            }
        }

        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;

        let mut stmt = conn.prepare(
            "SELECT b.id, b.type, b.title, p.url, b.dateAdded, b.lastModified, b.parent, b.position
             FROM moz_bookmarks b
             LEFT JOIN moz_places p ON b.fk = p.id
             WHERE b.type IN (1, 2, 3)
             ORDER BY b.parent, b.position",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(FirefoxRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                url: row.get(3)?,
                date_added: row.get(4)?,
                date_modified: row.get(5)?,
                parent: row.get(6)?,
                position: row.get::<_, i64>(7)?.max(0) as usize,
            })
        })?;

        let mut by_id = BTreeMap::new();
        let mut by_parent: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for row in rows {
            let row = row?;
            by_parent.entry(row.parent).or_default().push(row.id);
            by_id.insert(row.id, row);
        }

        let mut bookmark_count = 0;
        let mut folder_count = 0;
        let mut nodes = Vec::new();
        for (root_id, title, root_kind) in [
            (3, "Bookmarks Toolbar", BookmarkRootKind::Toolbar),
            (2, "Bookmarks Menu", BookmarkRootKind::BookmarkMenu),
            (5, "Other Bookmarks", BookmarkRootKind::Unfiled),
            (6, "Mobile Bookmarks", BookmarkRootKind::MobileBookmarks),
        ] {
            let root_children: Vec<_> = by_parent
                .get(&root_id)
                .into_iter()
                .flatten()
                .filter_map(|child_id| {
                    build_node(
                        *child_id,
                        &mut by_id,
                        &by_parent,
                        source,
                        &root_kind,
                        &[title.to_string()],
                        &mut bookmark_count,
                        &mut folder_count,
                    )
                })
                .collect();
            if !root_children.is_empty() {
                nodes.push(BookmarkNode::Folder(FolderNode {
                    id: format!("firefox-root-{root_id}"),
                    title: title.to_string(),
                    children: root_children,
                    source: source.clone(),
                    root_kind,
                    original_position: nodes.len(),
                    date_added: None,
                    date_modified: None,
                }));
                folder_count += 1;
            }
        }

        Ok((nodes, bookmark_count, folder_count))
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
                    let root = child.as_dictionary();
                    let root_id = root
                        .and_then(|node| node.get("WebBookmarkUUID"))
                        .and_then(|value| value.as_string());
                    let root_kind = match root_id {
                        Some("BookmarksBar") => BookmarkRootKind::BookmarkBar,
                        Some("BookmarksMenu") => BookmarkRootKind::BookmarkMenu,
                        _ => BookmarkRootKind::Custom(
                            root.and_then(|node| node.get("Title"))
                                .and_then(|value| value.as_string())
                                .or(root_id)
                                .unwrap_or("Safari Root")
                                .to_string(),
                        ),
                    };
                    let (child_nodes, bm, fld) =
                        Self::parse_safari_node(child, source, &root_kind, &[], idx)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_precedes_discovery_and_snapshot_output() -> Result<()> {
        let cancellation = super::super::cancellation::Cancellation::default();
        cancellation.cancel();
        let error =
            BookmarkCollector::discover_installations_from_home_cancellable(None, &cancellation)
                .unwrap_err();
        assert!(error.is::<super::super::cancellation::Cancelled>());
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("not-created");
        let error =
            BookmarkCollector::collect_all_cancellable(&output, false, &cancellation).unwrap_err();
        assert!(error.is::<super::super::cancellation::Cancelled>());
        assert!(!output.exists());
        let error = BookmarkCollector::collect_discovered_installations_cancellable(
            &output,
            false,
            Vec::new(),
            &cancellation,
        )
        .unwrap_err();
        assert!(error.is::<super::super::cancellation::Cancelled>());
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn full_discovery_requires_home() {
        let error = BookmarkCollector::discover_installations_from_home(None).unwrap_err();
        assert!(error.to_string().contains("HOME"));
    }

    #[test]
    fn installation_discovery_propagates_application_support_read_errors() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("Library")).unwrap();
        fs::write(
            temp.path().join("Library/Application Support"),
            b"not a directory",
        )
        .unwrap();

        let error =
            BookmarkCollector::discover_installations_from_home(Some(temp.path().to_path_buf()))
                .unwrap_err();
        assert!(
            error.to_string().contains("Application Support")
                || error.to_string().contains("browser installation candidate")
        );
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_unclassified_directory_does_not_abort_browser_discovery() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir()?;
        let support = temp.path().join("Library/Application Support");
        let chrome = support.join("Google/Chrome");
        fs::create_dir_all(chrome.join("Default"))?;
        fs::write(chrome.join("Default/Bookmarks"), b"{}")?;
        let unrelated = support.join("discord");
        fs::create_dir(&unrelated)?;
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o000))?;
        if fs::read_dir(&unrelated).is_ok() {
            fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o700))?;
            return Ok(()); // Root can bypass this fixture's Unix permission barrier.
        }
        let result =
            BookmarkCollector::discover_installations_from_home(Some(temp.path().to_path_buf()));
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o700))?;
        let (installations, issues) = result?;
        assert!(installations.iter().any(|item| item.root_path == chrome));
        assert!(!installations.iter().any(|item| item.root_path == unrelated));
        assert!(issues.iter().any(
            |issue| issue["path"] == unrelated.to_string_lossy().as_ref()
                && issue["browser_identity_verified"] == false
                && issue.get("bookmark_count").is_none()
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn blocked_vendor_child_does_not_hide_readable_nested_browser() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir()?;
        let vendor = temp.path().join("Library/Application Support/Vendor");
        let browser = vendor.join("NestedBrowser");
        fs::create_dir_all(browser.join("Default"))?;
        fs::write(browser.join("Default/Bookmarks"), b"{}")?;
        let blocked = vendor.join("BlockedApp");
        fs::create_dir(&blocked)?;
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000))?;
        let result =
            BookmarkCollector::discover_installations_from_home(Some(temp.path().to_path_buf()));
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))?;
        let (installations, _) = result?;
        assert!(installations
            .iter()
            .any(|installation| installation.root_path == browser));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn denied_known_browser_is_reported_while_readable_bookmarks_are_collected() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir()?;
        let support = temp.path().join("Library/Application Support");
        let denied = support.join("Google/Chrome");
        fs::create_dir_all(&denied)?;
        let brave = support.join("BraveSoftware/Brave-Browser/Default");
        fs::create_dir_all(&brave)?;
        fs::write(brave.join("Bookmarks"), br#"{"roots":{"bookmark_bar":{"id":"1","type":"folder","name":"Bar","children":[{"id":"2","type":"url","name":"Readable","url":"https://fixture.invalid/keep"}]}}}"#)?;
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000))?;
        if fs::read_dir(&denied).is_ok() {
            fs::set_permissions(&denied, fs::Permissions::from_mode(0o700))?;
            return Ok(());
        }
        let result = (|| {
            let (installations, _) = BookmarkCollector::discover_installations_from_home(Some(
                temp.path().to_path_buf(),
            ))?;
            BookmarkCollector::collect_discovered_installations(
                &temp.path().join("output"),
                false,
                installations,
            )
        })();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700))?;
        let (_, inventory, _) = result?;
        assert_eq!(inventory.total_bookmarks, 1);
        let chrome = inventory
            .installations
            .iter()
            .find(|item| item.installation.name == "Google Chrome")
            .unwrap();
        assert!(chrome.profiles.is_empty());
        assert!(chrome
            .discovery_error
            .as_deref()
            .is_some_and(|error| error.contains("Permission denied")));
        Ok(())
    }

    #[test]
    fn discovers_safari_and_technology_preview_as_separate_installations() {
        let temp = tempfile::tempdir().unwrap();
        let safari = temp.path().join("Library/Safari/Bookmarks.plist");
        let technology_preview = temp.path().join(
            "Library/Containers/com.apple.SafariTechnologyPreview/Data/Library/Safari/Bookmarks.plist",
        );
        fs::create_dir_all(safari.parent().unwrap()).unwrap();
        fs::create_dir_all(technology_preview.parent().unwrap()).unwrap();
        fs::write(&safari, b"safari").unwrap();
        fs::write(&technology_preview, b"technology-preview").unwrap();
        fs::create_dir_all(temp.path().join("Library/Application Support")).unwrap();

        let (installations, _) =
            BookmarkCollector::discover_installations_from_home(Some(temp.path().to_path_buf()))
                .unwrap();
        let safari_installations: Vec<_> = installations
            .iter()
            .filter(|installation| installation.kind == BrowserKind::Safari)
            .collect();

        assert_eq!(safari_installations.len(), 2);
        assert_eq!(safari_installations[0].name, "Safari");
        assert_eq!(safari_installations[0].channel, None);
        assert_eq!(safari_installations[0].root_path, safari);
        assert_eq!(safari_installations[1].name, "Safari Technology Preview");
        assert_eq!(
            safari_installations[1].channel.as_deref(),
            Some("Technology Preview")
        );
        assert_eq!(safari_installations[1].root_path, technology_preview);
        assert_eq!(
            BookmarkCollector::discover_profiles(safari_installations[0]).unwrap()[0].path,
            safari_installations[0].root_path
        );
        assert_eq!(
            BookmarkCollector::discover_profiles(safari_installations[1]).unwrap()[0].path,
            safari_installations[1].root_path
        );
    }

    #[test]
    fn safari_roots_keep_their_actual_origin_and_folder_path() {
        fn folder(id: &str, title: &str, url: &str) -> plist::Value {
            let mut leaf = plist::Dictionary::new();
            leaf.insert("WebBookmarkType".into(), "WebBookmarkTypeLeaf".into());
            leaf.insert("WebBookmarkUUID".into(), format!("{id}-leaf").into());
            leaf.insert("Title".into(), title.into());
            leaf.insert("URLString".into(), url.into());

            let mut root = plist::Dictionary::new();
            root.insert("WebBookmarkType".into(), "WebBookmarkTypeList".into());
            root.insert("WebBookmarkUUID".into(), id.into());
            root.insert("Title".into(), title.into());
            root.insert(
                "Children".into(),
                vec![plist::Value::Dictionary(leaf)].into(),
            );
            plist::Value::Dictionary(root)
        }

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Bookmarks.plist");
        let mut document = plist::Dictionary::new();
        document.insert(
            "Children".into(),
            vec![
                folder("BookmarksBar", "BookmarksBar", "https://bar.example"),
                folder("BookmarksMenu", "BookmarksMenu", "https://menu.example"),
                folder("custom-root", "Personal", "https://personal.example"),
                folder(
                    "reading-list",
                    "com.apple.ReadingList",
                    "https://reading.example",
                ),
                folder("history", "History", "https://history.example"),
            ]
            .into(),
        );
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &plist::Value::Dictionary(document)).unwrap();
        fs::write(&path, bytes).unwrap();
        let source = SourceDescriptor {
            browser: BrowserKind::Safari,
            browser_channel: None,
            profile_id: "default".into(),
            profile_name: "Safari".into(),
            source_path: path.clone(),
        };

        let (nodes, count, _) = BookmarkCollector::parse_safari_plist(&path, &source).unwrap();
        assert_eq!(count, 3);
        for (node, (kind, path)) in nodes.iter().zip([
            (BookmarkRootKind::BookmarkBar, "BookmarksBar"),
            (BookmarkRootKind::BookmarkMenu, "BookmarksMenu"),
            (BookmarkRootKind::Custom("Personal".into()), "Personal"),
        ]) {
            let BookmarkNode::Folder(folder) = node else {
                panic!("Safari root should remain a folder");
            };
            assert_eq!(folder.root_kind, kind);
            let BookmarkNode::Bookmark(bookmark) = &folder.children[0] else {
                panic!("Safari root should contain its URL");
            };
            assert_eq!(bookmark.root_kind, kind);
            assert_eq!(bookmark.folder_path, vec![path.to_string()]);
        }
    }

    #[test]
    fn profile_discovery_propagates_directory_read_errors() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("not-a-directory");
        fs::write(&root, b"file").unwrap();
        let installation = BrowserInstallation {
            name: "Broken Chromium".into(),
            kind: BrowserKind::Chromium("Chromium".into()),
            channel: None,
            root_path: root,
        };

        let error = BookmarkCollector::discover_profiles(&installation).unwrap_err();
        assert!(format!("{error:#}").contains("not-a-directory"));
    }

    #[test]
    fn full_collection_rejects_all_zero_profile_installations() {
        let temp = tempfile::tempdir().unwrap();
        let empty_root = temp.path().join("empty-browser");
        fs::create_dir(&empty_root).unwrap();
        let installation = BrowserInstallation {
            name: "Empty Chromium".into(),
            kind: BrowserKind::Chromium("Chromium".into()),
            channel: None,
            root_path: empty_root,
        };

        let error = BookmarkCollector::collect_discovered_installations(
            &temp.path().join("output"),
            false,
            vec![installation],
        )
        .unwrap_err();
        assert!(error.to_string().contains("no readable browser profiles"));
    }

    #[test]
    fn discovers_named_chromium_profile_by_bookmarks_file() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("Work Account");
        fs::create_dir(&profile).unwrap();
        fs::write(profile.join("Bookmarks"), "{}").unwrap();
        let installation = BrowserInstallation {
            name: "Test Chromium".into(),
            kind: BrowserKind::Chromium("Chromium".into()),
            channel: None,
            root_path: temp.path().to_path_buf(),
        };

        let profiles = BookmarkCollector::discover_profiles(&installation).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "Work Account");
    }

    #[test]
    fn discovers_reading_list_only_chromium_profile() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let profile = temp.path().join("Profile 16");
        fs::create_dir_all(profile.join("Sync Data/LevelDB"))?;
        fs::write(
            profile.join("Sync Data/LevelDB/CURRENT"),
            "MANIFEST-000001\n",
        )?;
        let installation = BrowserInstallation {
            name: "Test Chromium".into(),
            kind: BrowserKind::Chromium("Chromium".into()),
            channel: None,
            root_path: temp.path().to_path_buf(),
        };
        assert!(BookmarkCollector::discover_profiles(&installation)?.is_empty());
        let profiles = BookmarkCollector::discover_reading_list_profiles(&installation)?;
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].path, profile);
        Ok(())
    }

    #[test]
    fn discovers_unknown_reading_list_only_installations() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let support = temp.path().join("Library/Application Support");
        let roots = [
            support.join("UnknownChromium"),
            support.join("Vendor/NestedChromium"),
            support.join("SingleProfileChromium"),
        ];
        for (index, root) in roots.iter().enumerate() {
            let profile = if index == 2 {
                root.clone()
            } else {
                root.join("Profile 1")
            };
            fs::create_dir_all(profile.join("Sync Data/LevelDB"))?;
            fs::write(
                profile.join("Sync Data/LevelDB/CURRENT"),
                "MANIFEST-000001\n",
            )?;
        }
        let (installations, _) =
            BookmarkCollector::discover_installations_from_home(Some(temp.path().to_path_buf()))?;
        assert_eq!(installations.len(), roots.len());
        for root in roots {
            let installation = installations
                .iter()
                .find(|item| item.root_path == root)
                .unwrap();
            assert!(BookmarkCollector::discover_profiles(installation)?.is_empty());
            assert_eq!(
                BookmarkCollector::discover_reading_list_profiles(installation)?.len(),
                1
            );
        }
        Ok(())
    }

    #[test]
    fn discovers_root_chromium_and_external_firefox_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let chromium_root = temp.path().join("Opera");
        fs::create_dir(&chromium_root).unwrap();
        fs::write(chromium_root.join("Bookmarks"), "{}").unwrap();
        let chromium = BrowserInstallation {
            name: "Test Chromium".into(),
            kind: BrowserKind::Chromium("Chromium".into()),
            channel: None,
            root_path: chromium_root,
        };
        let profiles = BookmarkCollector::discover_profiles(&chromium).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].path, chromium.root_path);

        let firefox_root = temp.path().join("Firefox");
        let external = temp.path().join("External Profile");
        fs::create_dir_all(&firefox_root).unwrap();
        fs::create_dir_all(&external).unwrap();
        rusqlite::Connection::open(external.join("places.sqlite")).unwrap();
        fs::write(
            firefox_root.join("profiles.ini"),
            format!(
                "[Profile0]\nName=External Work\nIsRelative=0\nPath={}\n",
                external.display()
            ),
        )
        .unwrap();
        let firefox = BrowserInstallation {
            name: "Test Firefox".into(),
            kind: BrowserKind::Firefox("Firefox".into()),
            channel: None,
            root_path: firefox_root,
        };
        let profiles = BookmarkCollector::discover_profiles(&firefox).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "External Work");
        assert_eq!(profiles[0].path, external);
    }

    #[test]
    fn firefox_parser_preserves_nested_and_mobile_paths() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("places.sqlite");
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT);
             CREATE TABLE moz_bookmarks (
                 id INTEGER PRIMARY KEY, type INTEGER NOT NULL, fk INTEGER,
                 parent INTEGER NOT NULL, position INTEGER NOT NULL, title TEXT,
                 dateAdded INTEGER, lastModified INTEGER
             );
             INSERT INTO moz_places VALUES (1, 'https://nested.example');
             INSERT INTO moz_places VALUES (2, 'https://mobile.example');
             INSERT INTO moz_bookmarks VALUES (10, 2, NULL, 3, 0, 'Projects', NULL, NULL);
             INSERT INTO moz_bookmarks VALUES (11, 1, 1, 10, 0, 'Nested', NULL, NULL);
             INSERT INTO moz_bookmarks VALUES (12, 1, 2, 6, 0, 'Mobile', NULL, NULL);",
        )
        .unwrap();
        drop(db);
        let source = SourceDescriptor {
            browser: BrowserKind::Firefox("Firefox".into()),
            browser_channel: None,
            profile_id: "test".into(),
            profile_name: "Test".into(),
            source_path: db_path.clone(),
        };

        let (nodes, bookmark_count, _) =
            BookmarkCollector::parse_firefox_sqlite(&db_path, &source).unwrap();
        let mut paths = BTreeMap::new();
        fn collect(nodes: &[BookmarkNode], paths: &mut BTreeMap<String, Vec<String>>) {
            for node in nodes {
                match node {
                    BookmarkNode::Bookmark(bookmark) => {
                        paths.insert(bookmark.raw_url.clone(), bookmark.folder_path.clone());
                    }
                    BookmarkNode::Folder(folder) => collect(&folder.children, paths),
                    BookmarkNode::Separator(_) => {}
                }
            }
        }
        collect(&nodes, &mut paths);

        assert_eq!(bookmark_count, 2);
        assert_eq!(
            paths["https://nested.example"],
            ["Bookmarks Toolbar", "Projects"]
        );
        assert_eq!(paths["https://mobile.example"], ["Mobile Bookmarks"]);
    }

    #[cfg(unix)]
    #[test]
    fn deduplicates_installations_that_resolve_to_the_same_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("LibreWolf");
        let alias = temp.path().join("librewolf-alias");
        fs::create_dir(&root).unwrap();
        symlink(&root, &alias).unwrap();
        let mut installations = vec![
            BrowserInstallation {
                name: "LibreWolf".into(),
                kind: BrowserKind::Firefox("LibreWolf".into()),
                channel: None,
                root_path: root,
            },
            BrowserInstallation {
                name: "librewolf".into(),
                kind: BrowserKind::Firefox("librewolf".into()),
                channel: None,
                root_path: alias,
            },
        ];

        BookmarkCollector::deduplicate_installations(&mut installations);

        assert_eq!(installations.len(), 1);
        assert_eq!(installations[0].name, "LibreWolf");
    }

    #[test]
    fn onion_filter_is_recursive_and_hostname_specific() {
        let source = SourceDescriptor {
            browser: BrowserKind::Firefox("Test".into()),
            browser_channel: None,
            profile_id: "profile".into(),
            profile_name: "Profile".into(),
            source_path: PathBuf::from("places.sqlite"),
        };
        let bookmark = |id: &str, url: &str| {
            BookmarkNode::Bookmark(BookmarkOccurrence {
                id: id.into(),
                title: id.into(),
                raw_url: url.into(),
                source: source.clone(),
                root_kind: BookmarkRootKind::OtherBookmarks,
                folder_path: vec!["Nested".into()],
                original_position: 0,
                date_added: None,
                date_modified: None,
                metadata: BTreeMap::new(),
            })
        };
        let mut nodes = vec![BookmarkNode::Folder(FolderNode {
            id: "folder".into(),
            title: "Nested".into(),
            children: vec![
                bookmark("blocked", "http://hidden.example.onion/post"),
                bookmark("kept", "https://example.com/path/.onion"),
            ],
            source,
            root_kind: BookmarkRootKind::OtherBookmarks,
            original_position: 0,
            date_added: None,
            date_modified: None,
        })];

        assert_eq!(BookmarkCollector::remove_onion_nodes(&mut nodes), 1);
        let BookmarkNode::Folder(folder) = &nodes[0] else {
            panic!("folder must be preserved");
        };
        assert_eq!(folder.children.len(), 1);
    }
}
