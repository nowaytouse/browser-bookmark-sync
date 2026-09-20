use anyhow::{bail, Context, Result};
use prost::Message;
use rusty_leveldb::{LdbIterator, Options, DB};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, PartialEq, Message)]
struct ReadingListLocal {
    #[prost(string, optional, tag = "1")]
    entry_id: Option<String>,
    #[prost(string, optional, tag = "2")]
    title: Option<String>,
    #[prost(string, optional, tag = "3")]
    url: Option<String>,
    #[prost(int64, optional, tag = "4")]
    creation_time_us: Option<i64>,
}

pub(super) fn read_store(source: &Path) -> Result<Vec<crate::browsers::ReadingListItem>> {
    let snapshot = snapshot_store(source)?;
    let source_before = snapshot.source_hash.clone();
    let source_files_before = snapshot.source_files.clone();
    let result = (|| {
        let mut options = Options::default();
        options.create_if_missing = false;
        options.error_if_exists = false;
        options.paranoid_checks = true;
        let mut db =
            DB::open(snapshot.dir.path(), options).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut iter = db.new_iter().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let prefixes: [&[u8]; 2] = [b"A-reading_list-dt-", b"reading_list-dt-"];
        iter.seek(b"A-reading_list-dt-");
        let mut items = Vec::new();
        while iter.valid() {
            let Some((key, value)) = iter.current() else {
                bail!("LevelDB iterator yielded an invalid record");
            };
            let Some(prefix) = prefixes.iter().find(|prefix| key.starts_with(prefix)) else {
                if !iter.advance() {
                    break;
                }
                continue;
            };
            let key = String::from_utf8(key[prefix.len()..].to_vec())
                .context("Reading List key is not UTF-8")?;
            let proto = ReadingListLocal::decode(value)
                .context("invalid ReadingListLocal protobuf record")?;
            let Some(url) = proto.url.filter(|v| !v.is_empty()) else {
                bail!("ReadingListLocal record has no URL");
            };
            if key != url {
                bail!("ReadingListLocal key/URL mismatch");
            }
            items.push(crate::browsers::ReadingListItem {
                url,
                title: proto.title.unwrap_or_default(),
                date_added: proto.creation_time_us.map(|micros| micros / 1_000),
            });
            if !iter.advance() {
                break;
            }
        }
        Ok(items)
    })();
    verify_source_unchanged(source, &source_files_before, &source_before)?;
    result
}

struct Snapshot {
    dir: TempDir,
    source_files: Vec<PathBuf>,
    source_hash: Vec<u8>,
}

fn snapshot_store(source: &Path) -> Result<Snapshot> {
    if !source.is_dir() {
        bail!(
            "Chromium Reading List source is not a directory: {}",
            source.display()
        );
    }
    let files = store_files(source)?;
    if files.is_empty() {
        bail!("Chromium LevelDB snapshot has no recognized data files");
    }
    let before = hash_files(&files)?;
    let dest = tempfile::tempdir().context("create LevelDB snapshot directory")?;
    for path in &files {
        let name = path.file_name().context("LevelDB file has no name")?;
        fs::copy(path, dest.path().join(name))
            .with_context(|| format!("snapshot {}", path.display()))?;
    }
    verify_source_unchanged(source, &files, &before)?;
    if hash_files(&store_files(dest.path())?)? != before {
        bail!("Chromium LevelDB snapshot differs from source baseline");
    }
    Ok(Snapshot {
        dir: dest,
        source_files: files,
        source_hash: before,
    })
}

fn verify_source_unchanged(
    source: &Path,
    before_files: &[PathBuf],
    before_hash: &[u8],
) -> Result<()> {
    let source_files = store_files(source)?;
    if file_names(&source_files) != file_names(before_files)
        || hash_files(&source_files)? != before_hash
    {
        bail!("Chromium LevelDB changed while reading snapshot");
    }
    Ok(())
}

fn store_files(source: &Path) -> Result<Vec<PathBuf>> {
    let mut total = 0u64;
    let mut files = Vec::new();
    for entry in fs::read_dir(source).context("read LevelDB directory")? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() {
            bail!(
                "Chromium LevelDB candidate is not a regular file: {}",
                path.display()
            );
        }
        let allowed = name == "CURRENT"
            || numeric_name(name, "MANIFEST-", "")
            || numeric_name(name, "", ".log")
            || numeric_name(name, "", ".ldb")
            || numeric_name(name, "", ".sst");
        if !allowed {
            continue;
        }
        total = total
            .checked_add(metadata.len())
            .ok_or_else(|| anyhow::anyhow!("LevelDB snapshot size overflow"))?;
        if total > MAX_SNAPSHOT_BYTES {
            bail!(
                "Chromium LevelDB snapshot exceeds {} bytes",
                MAX_SNAPSHOT_BYTES
            );
        }
        files.push(path);
    }
    files.sort();
    let current = source.join("CURRENT");
    let current_value = fs::read_to_string(&current).context("read LevelDB CURRENT")?;
    let manifest = current_value.trim();
    if !numeric_name(manifest, "MANIFEST-", "") || !source.join(manifest).is_file() {
        bail!("Chromium LevelDB CURRENT does not reference a valid manifest");
    }
    Ok(files)
}

fn numeric_name(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|n| n.strip_suffix(suffix))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn file_names(files: &[PathBuf]) -> Vec<std::ffi::OsString> {
    files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_os_string()))
        .collect()
}

fn hash_files(files: &[PathBuf]) -> Result<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();
    for path in files {
        let name = path.file_name().context("LevelDB file has no name")?;
        hasher.update(&(fs::metadata(path)?.len()).to_le_bytes());
        hasher.update(name.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        let bytes = fs::read(path).with_context(|| format!("hash {}", path.display()))?;
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_db(path: &Path, key: &[u8], value: &[u8]) -> Result<()> {
        let mut options = Options::default();
        options.create_if_missing = true;
        let mut db = DB::open(path, options).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        db.put(key, value)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        db.flush().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        db.close().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    }

    #[test]
    fn reads_only_reading_list_namespace_and_preserves_source() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("LevelDB");
        fs::create_dir(&source)?;
        let url = "https://fixture.invalid/read";
        let value = ReadingListLocal {
            url: Some(url.to_string()),
            title: Some("fixture".to_string()),
            creation_time_us: Some(42_000),
            ..Default::default()
        }
        .encode_to_vec();
        put_db(&source, format!("reading_list-dt-{url}").as_bytes(), &value)?;
        let account_url = "https://fixture.invalid/account";
        let account_value = ReadingListLocal {
            url: Some(account_url.to_string()),
            title: Some("account".to_string()),
            ..Default::default()
        }
        .encode_to_vec();
        put_db(
            &source,
            format!("A-reading_list-dt-{account_url}").as_bytes(),
            &account_value,
        )?;
        put_db(
            &source,
            b"bookmarks-dt-should-not-leak",
            b"not reading list",
        )?;
        put_db(&source, b"reading_list-md-metadata", b"metadata")?;
        let before = hash_files(&store_files(&source)?)?;
        let items = read_store(&source)?;
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|item| item.url == url && item.title == "fixture"));
        assert!(items
            .iter()
            .any(|item| item.url == account_url && item.title == "account"));
        assert_eq!(
            items
                .iter()
                .find(|item| item.url == url)
                .unwrap()
                .date_added,
            Some(42)
        );
        assert_eq!(hash_files(&store_files(&source)?)?, before);
        Ok(())
    }

    #[test]
    fn rejects_corrupt_reading_list_record() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("LevelDB");
        fs::create_dir(&source)?;
        put_db(
            &source,
            b"reading_list-dt-https://fixture.invalid/bad",
            b"bad",
        )?;
        assert!(read_store(&source).is_err());
        Ok(())
    }

    #[test]
    fn deleted_record_is_not_returned() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("LevelDB");
        fs::create_dir(&source)?;
        let mut options = Options::default();
        options.create_if_missing = true;
        let mut db = DB::open(&source, options).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let value = ReadingListLocal {
            url: Some("https://fixture.invalid/deleted".to_string()),
            ..Default::default()
        }
        .encode_to_vec();
        db.put(b"reading_list-dt-https://fixture.invalid/deleted", &value)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        db.delete(b"reading_list-dt-https://fixture.invalid/deleted")
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        db.flush().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        db.close().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        assert!(read_store(&source)?.is_empty());
        Ok(())
    }
}
