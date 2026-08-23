use anyhow::{Context, Result};
use blake3::Hasher;
use chrono::Utc;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tracing::{info, warn};

use crate::browsers::BrowserType;
use crate::data_types::models::{BrowserProfile, ProfileSnapshot};

const STABLE_COPY_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    len: u64,
    modified: std::time::SystemTime,
    hash: String,
}

static OPTIMISTIC_WRITES: OnceLock<Mutex<HashMap<PathBuf, FileFingerprint>>> = OnceLock::new();

pub(crate) struct OptimisticWriteGuard {
    path: PathBuf,
}

impl Drop for OptimisticWriteGuard {
    fn drop(&mut self) {
        if let Ok(mut writes) = optimistic_writes().lock() {
            writes.remove(&self.path);
        }
    }
}

fn optimistic_writes() -> &'static Mutex<HashMap<PathBuf, FileFingerprint>> {
    OPTIMISTIC_WRITES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn write_key(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn optimistic_write_guard(
    path: &Path,
    expected: &FileFingerprint,
) -> Result<OptimisticWriteGuard> {
    ensure_file_unchanged(path, expected)?;
    let path = write_key(path);
    let mut writes = optimistic_writes()
        .lock()
        .map_err(|_| anyhow::anyhow!("Optimistic write registry is unavailable"))?;
    anyhow::ensure!(
        !writes.contains_key(&path),
        "Another bookmark write is already pending for {}",
        path.display()
    );
    writes.insert(path.clone(), expected.clone());
    Ok(OptimisticWriteGuard { path })
}

fn registered_expectation(path: &Path) -> Result<Option<FileFingerprint>> {
    optimistic_writes()
        .lock()
        .map(|writes| writes.get(&write_key(path)).cloned())
        .map_err(|_| anyhow::anyhow!("Optimistic write registry is unavailable"))
}

fn ensure_file_unchanged(path: &Path, expected: &FileFingerprint) -> Result<()> {
    anyhow::ensure!(
        &file_fingerprint(path)? == expected,
        "Refusing to overwrite {}: source file changed since it was read",
        path.display()
    );
    Ok(())
}

pub fn atomic_write_file(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Invalid output path: {}", path.display()))?;
    let name = path
        .file_name()
        .with_context(|| format!("Invalid output path: {}", path.display()))?
        .to_string_lossy();
    let expected = match registered_expectation(path)? {
        Some(expected) => Some(expected),
        None if path.exists() => Some(file_fingerprint(path)?),
        None => None,
    };
    let permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        if let Some(expected) = expected.as_ref() {
            ensure_file_unchanged(path, expected)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(data)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        if let Some(expected) = expected.as_ref() {
            ensure_file_unchanged(path, expected)?;
        } else {
            anyhow::ensure!(
                !path.exists(),
                "Refusing to overwrite {}: file appeared during write",
                path.display()
            );
        }
        fs::rename(&temp, path)?;
        let _ = fs::File::open(parent).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Safely create a read-only snapshot copy of a browser profile
pub fn create_profile_snapshot(
    profile: &BrowserProfile,
    dest_dir: &Path,
) -> Result<ProfileSnapshot> {
    let snapshot_folder_name = format!("{}_{}", profile.id.replace('/', "_"), uuid::Uuid::new_v4());
    let snapshot_dir = dest_dir.join(&snapshot_folder_name);
    fs::create_dir_all(&snapshot_dir)
        .with_context(|| format!("Failed to create snapshot directory {snapshot_dir:?}"))?;

    let mut hashes = BTreeMap::new();

    // Copy relevant bookmark and DB files safely
    if profile.path.exists() {
        if profile.path.is_file() {
            // e.g. Safari Bookmarks.plist or single file
            if let Some(file_name) = profile.path.file_name() {
                let target_path = snapshot_dir.join(file_name);
                copy_stable_file(&profile.path, &target_path)?;
                hashes.insert(
                    file_name.to_string_lossy().to_string(),
                    calculate_file_hash(&target_path)?,
                );
            }
        } else if profile.path.is_dir() {
            // Copy files like Bookmarks, places.sqlite, places.sqlite-wal, Local State, etc.
            let targets = [
                "Bookmarks",
                "Bookmarks.bak",
                "places.sqlite",
                "favicons.sqlite",
                "Local State",
                "Preferences",
            ];

            for entry in targets {
                let src_file = profile.path.join(entry);
                if src_file.exists() && src_file.is_file() {
                    let dst_file = snapshot_dir.join(entry);
                    if entry == "places.sqlite" {
                        let source = rusqlite::Connection::open_with_flags(
                            &src_file,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                        )?;
                        source.execute("VACUUM main INTO ?1", [dst_file.to_string_lossy()])?;
                    } else if matches!(
                        entry,
                        "Bookmarks" | "Bookmarks.bak" | "Local State" | "Preferences"
                    ) {
                        copy_stable_file(&src_file, &dst_file)?;
                    } else {
                        fs::copy(&src_file, &dst_file)
                            .with_context(|| format!("Failed to snapshot {src_file:?}"))?;
                    }
                    hashes.insert(entry.to_string(), calculate_file_hash(&dst_file)?);
                }
            }
        }
    }

    anyhow::ensure!(
        !hashes.is_empty(),
        "Profile {:?} had no readable bookmark data",
        profile.path
    );

    Ok(ProfileSnapshot {
        original_profile: profile.clone(),
        snapshot_path: snapshot_dir,
        created_at: Utc::now(),
        blake3_hashes: hashes,
    })
}

pub(crate) fn copy_stable_file(source: &Path, destination: &Path) -> Result<()> {
    let mut last_error = None;

    for attempt in 0..STABLE_COPY_ATTEMPTS {
        let result = (|| -> Result<()> {
            let before = file_fingerprint(source)?;
            fs::copy(source, destination)
                .with_context(|| format!("Failed to snapshot {source:?}"))?;
            let after = file_fingerprint(source)?;
            anyhow::ensure!(
                before == after,
                "Source file changed while snapshotting {source:?}"
            );
            anyhow::ensure!(
                calculate_file_hash(destination)? == before.hash,
                "Snapshot content differs from stable source {source:?}"
            );
            Ok(())
        })();

        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = fs::remove_file(destination);
                last_error = Some(error);
                if attempt + 1 < STABLE_COPY_ATTEMPTS {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Stable snapshot failed for {source:?}")))
}

pub(crate) fn file_fingerprint(path: &Path) -> Result<FileFingerprint> {
    let metadata = fs::metadata(path)?;
    let len = metadata.len();
    let modified = metadata.modified()?;
    let hash = calculate_file_hash(path)?;
    let after = fs::metadata(path)?;
    anyhow::ensure!(
        len == after.len() && modified == after.modified()?,
        "Source file changed while reading {path:?}"
    );
    Ok(FileFingerprint {
        len,
        modified,
        hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_live_firefox_database_consistently() {
        let temp = tempfile::tempdir().unwrap();
        let profile_dir = temp.path().join("profile");
        fs::create_dir(&profile_dir).unwrap();
        let db_path = profile_dir.join("places.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE bookmark_test (url TEXT NOT NULL);
             INSERT INTO bookmark_test VALUES ('https://example.com/post');",
        )
        .unwrap();
        let profile = BrowserProfile {
            id: "profile".into(),
            name: "Profile".into(),
            path: profile_dir,
        };
        let snapshot = create_profile_snapshot(&profile, &temp.path().join("snapshots")).unwrap();
        let copy =
            rusqlite::Connection::open(snapshot.snapshot_path.join("places.sqlite")).unwrap();
        let value: String = copy
            .query_row("SELECT url FROM bookmark_test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "https://example.com/post");
    }

    #[test]
    fn snapshots_stable_chromium_json_content() {
        let temp = tempfile::tempdir().unwrap();
        let profile_dir = temp.path().join("profile");
        fs::create_dir(&profile_dir).unwrap();
        fs::write(profile_dir.join("Bookmarks"), b"{\"roots\":{}}\n").unwrap();
        let profile = BrowserProfile {
            id: "profile".into(),
            name: "Profile".into(),
            path: profile_dir,
        };

        let snapshot = create_profile_snapshot(&profile, &temp.path().join("snapshots")).unwrap();
        assert_eq!(
            fs::read(snapshot.snapshot_path.join("Bookmarks")).unwrap(),
            b"{\"roots\":{}}\n"
        );
    }

    #[test]
    fn snapshots_stable_single_file_content() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("Bookmarks.plist");
        fs::write(&source, b"plist-content\n").unwrap();
        let profile = BrowserProfile {
            id: "safari".into(),
            name: "Safari".into(),
            path: source,
        };

        let snapshot = create_profile_snapshot(&profile, &temp.path().join("snapshots")).unwrap();
        assert_eq!(
            fs::read(snapshot.snapshot_path.join("Bookmarks.plist")).unwrap(),
            b"plist-content\n"
        );
    }

    #[test]
    fn optimistic_write_rejects_a_changed_bookmark_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Bookmarks");
        fs::write(&path, b"read-version").unwrap();
        let expected = file_fingerprint(&path).unwrap();
        let _guard = optimistic_write_guard(&path, &expected).unwrap();

        fs::write(&path, b"browser-new-version").unwrap();
        let error = atomic_write_file(&path, b"stale-cleanup-result").unwrap_err();

        assert!(error.to_string().contains("source file changed"));
        assert_eq!(fs::read(path).unwrap(), b"browser-new-version");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_private_file_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Bookmarks");
        fs::write(&path, b"before").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        atomic_write_file(&path, b"after").unwrap();

        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
    }
}

fn calculate_file_hash(file_path: &Path) -> Result<String> {
    let mut file = fs::File::open(file_path)?;
    let mut hasher = Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Force close browsers before sync if user explicitly requests writeback
pub fn close_browsers(browsers: &[BrowserType], force: bool) -> Result<()> {
    info!("🔒 Closing browsers before sync...");

    for browser in browsers {
        close_browser(browser, force)?;
    }

    thread::sleep(Duration::from_secs(1));
    info!("✅ All target browsers closed");

    Ok(())
}

fn close_browser(browser: &BrowserType, force: bool) -> Result<()> {
    let (app_name, process_name) = match browser {
        BrowserType::Safari => ("Safari", "Safari"),
        BrowserType::Chrome => ("Google Chrome", "Google Chrome"),
        BrowserType::Brave => ("Brave Browser", "Brave Browser"),
        BrowserType::BraveNightly => ("Brave Browser Nightly", "Brave Browser Nightly"),
        BrowserType::Waterfox => ("Waterfox", "waterfox-bin"),
        BrowserType::FirefoxNightly => ("Firefox Nightly", "firefox"),
        BrowserType::LibreWolf => ("LibreWolf", "librewolf"),
    };

    if !force {
        let script = format!("tell application \"{app_name}\" to quit");
        let output = Command::new("osascript").arg("-e").arg(&script).output();

        if output.is_ok() {
            info!("  ✅ {} closed gracefully", app_name);
            return Ok(());
        }
    }

    let output = Command::new("killall").arg("-9").arg(process_name).output();

    match output {
        Ok(_) => {
            info!("  ✅ {} force killed", app_name);
        }
        Err(e) => {
            warn!("  ⚠️  Failed to kill {}: {}", app_name, e);
        }
    }

    Ok(())
}

/// Parse browser list from comma-separated string
pub fn parse_browser_list(browsers_str: &str) -> Vec<BrowserType> {
    browsers_str
        .split(',')
        .filter_map(|s| {
            let s = s.trim().to_lowercase();
            match s.as_str() {
                "safari" => Some(BrowserType::Safari),
                "chrome" => Some(BrowserType::Chrome),
                "brave" => Some(BrowserType::Brave),
                "brave-nightly" | "bravenightly" => Some(BrowserType::BraveNightly),
                "waterfox" => Some(BrowserType::Waterfox),
                "firefox-nightly" | "firefoxnightly" => Some(BrowserType::FirefoxNightly),
                "librewolf" => Some(BrowserType::LibreWolf),
                _ => None,
            }
        })
        .collect()
}
