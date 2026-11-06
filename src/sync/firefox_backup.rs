use std::path::Path;

use anyhow::{Context, Result};
use lz4_flex::block::decompress_size_prepended;
use serde::Deserialize;

use crate::browsers::Bookmark;

const MOZLZ4_MAGIC: &[u8; 8] = b"mozLz40\0";
const MAX_DECOMPRESSED_BACKUP_BYTES: u32 = 128 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct BackupNode {
    #[serde(default)]
    guid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    children: Vec<BackupNode>,
    #[serde(default, rename = "dateAdded")]
    date_added: Option<i64>,
    #[serde(default, rename = "lastModified")]
    last_modified: Option<i64>,
    #[serde(default, rename = "type")]
    node_type: String,
    #[serde(default, rename = "typeCode")]
    type_code: Option<u8>,
}

impl BackupNode {
    fn into_bookmark(self) -> Result<Option<Bookmark>> {
        if self.node_type == "text/x-moz-place-separator" || self.type_code == Some(3) {
            return Ok(None);
        }

        if let Some(url) = self.uri {
            return Ok(Some(Bookmark {
                id: self.guid,
                title: self.title,
                url: Some(url),
                folder: false,
                children: Vec::new(),
                date_added: self.date_added,
                date_modified: self.last_modified,
            }));
        }

        if self.node_type == "text/x-moz-place-container"
            || self.type_code == Some(2)
            || !self.children.is_empty()
        {
            let children = self
                .children
                .into_iter()
                .map(Self::into_bookmark)
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect();
            return Ok(Some(Bookmark {
                id: self.guid,
                title: self.title,
                url: None,
                folder: true,
                children,
                date_added: self.date_added,
                date_modified: self.last_modified,
            }));
        }

        anyhow::bail!(
            "unsupported Firefox backup node type '{}' ({:?})",
            self.node_type,
            self.type_code
        )
    }
}

pub(super) fn import_jsonlz4(path: &Path) -> Result<Vec<Bookmark>> {
    let data = std::fs::read(path).with_context(|| {
        format!(
            "failed to read Firefox bookmark backup '{}'",
            path.display()
        )
    })?;
    anyhow::ensure!(
        data.starts_with(MOZLZ4_MAGIC),
        "invalid Firefox JSONLZ4 magic in '{}'",
        path.display()
    );
    anyhow::ensure!(
        data.len() >= MOZLZ4_MAGIC.len() + 4,
        "truncated Firefox JSONLZ4 backup '{}'",
        path.display()
    );
    let declared_size = u32::from_le_bytes(
        data[MOZLZ4_MAGIC.len()..MOZLZ4_MAGIC.len() + 4]
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid Firefox JSONLZ4 size header"))?,
    );
    anyhow::ensure!(
        declared_size <= MAX_DECOMPRESSED_BACKUP_BYTES,
        "Firefox JSONLZ4 backup '{}' declares {} bytes, above the {} byte safety limit",
        path.display(),
        declared_size,
        MAX_DECOMPRESSED_BACKUP_BYTES
    );

    let json = decompress_size_prepended(&data[MOZLZ4_MAGIC.len()..]).with_context(|| {
        format!(
            "failed to decompress Firefox JSONLZ4 backup '{}'",
            path.display()
        )
    })?;
    let root: BackupNode = serde_json::from_slice(&json).with_context(|| {
        format!(
            "failed to parse Firefox JSONLZ4 backup '{}'",
            path.display()
        )
    })?;

    root.children
        .into_iter()
        .map(BackupNode::into_bookmark)
        .collect::<Result<Vec<_>>>()
        .map(|nodes| nodes.into_iter().flatten().collect())
        .with_context(|| format!("failed to map Firefox JSONLZ4 backup '{}'", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lz4_flex::block::compress_prepend_size;
    use serde_json::json;

    fn write_fixture(path: &Path, json: serde_json::Value) {
        let mut data = MOZLZ4_MAGIC.to_vec();
        data.extend(compress_prepend_size(
            &serde_json::to_vec(&json).expect("serialize fixture"),
        ));
        std::fs::write(path, data).expect("write fixture");
    }

    #[test]
    fn imports_folder_and_bookmark_and_ignores_separator() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("bookmarks.jsonlz4");
        write_fixture(
            &path,
            json!({
                "guid": "root-guid",
                "title": "root",
                "type": "text/x-moz-place-container",
                "typeCode": 2,
                "children": [{
                    "guid": "folder-guid",
                    "title": "Folder",
                    "type": "text/x-moz-place-container",
                    "typeCode": 2,
                    "dateAdded": 10,
                    "lastModified": 20,
                    "children": [{
                        "guid": "bookmark-guid",
                        "title": "Example",
                        "uri": "https://example.com/page",
                        "type": "text/x-moz-place",
                        "typeCode": 1,
                        "dateAdded": 30,
                        "lastModified": 40
                    }, {
                        "guid": "separator-guid",
                        "type": "text/x-moz-place-separator",
                        "typeCode": 3
                    }]
                }]
            }),
        );

        let bookmarks = import_jsonlz4(&path).expect("import backup");
        assert_eq!(bookmarks.len(), 1);
        let folder = &bookmarks[0];
        assert_eq!(folder.id, "folder-guid");
        assert_eq!(folder.title, "Folder");
        assert!(folder.folder);
        assert_eq!(folder.date_added, Some(10));
        assert_eq!(folder.date_modified, Some(20));
        assert_eq!(folder.children.len(), 1);

        let bookmark = &folder.children[0];
        assert_eq!(bookmark.id, "bookmark-guid");
        assert_eq!(bookmark.title, "Example");
        assert_eq!(bookmark.url.as_deref(), Some("https://example.com/page"));
        assert!(!bookmark.folder);
        assert_eq!(bookmark.date_added, Some(30));
        assert_eq!(bookmark.date_modified, Some(40));
    }

    #[test]
    fn rejects_bad_magic_with_path_context() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("bad.jsonlz4");
        std::fs::write(&path, b"not-a-firefox-backup").expect("write fixture");

        let error = import_jsonlz4(&path).expect_err("bad magic must fail");
        let message = format!("{error:#}");
        assert!(message.contains("invalid Firefox JSONLZ4 magic"));
        assert!(message.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn rejects_oversized_declared_payload_before_decompression() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("oversized.jsonlz4");
        let mut data = MOZLZ4_MAGIC.to_vec();
        data.extend(u32::MAX.to_le_bytes());
        std::fs::write(&path, data).expect("write fixture");

        let error = import_jsonlz4(&path).expect_err("oversized declaration must fail closed");
        assert!(format!("{error:#}").contains("safety limit"));
    }
}
