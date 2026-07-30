//! Unified Internal Data Models for High-Fidelity Bookmark Processing

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// High-level browser family classification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrowserKind {
    Chromium(String),
    Firefox(String),
    Safari,
}

impl std::fmt::Display for BrowserKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chromium(name) => write!(f, "Chromium({name})"),
            Self::Firefox(name) => write!(f, "Firefox({name})"),
            Self::Safari => write!(f, "Safari"),
        }
    }
}

/// Source metadata identifying where a bookmark originated
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub browser: BrowserKind,
    pub browser_channel: Option<String>,
    pub profile_id: String,
    pub profile_name: String,
    pub source_path: PathBuf,
}

/// Detected installed browser instance
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserInstallation {
    pub name: String,
    pub kind: BrowserKind,
    pub channel: Option<String>,
    pub root_path: PathBuf,
}

/// Discovered user profile within a browser installation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserProfile {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

/// Isolated read-only copy of a browser profile
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSnapshot {
    pub original_profile: BrowserProfile,
    pub snapshot_path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub blake3_hashes: BTreeMap<String, String>,
}

/// Standardized browser bookmark root folder kinds
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BookmarkRootKind {
    BookmarkBar,
    OtherBookmarks,
    MobileBookmarks,
    BookmarkMenu,
    Toolbar,
    Unfiled,
    ReadingList,
    Favorites,
    Custom(String),
}

impl std::fmt::Display for BookmarkRootKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BookmarkBar => write!(f, "Bookmark Bar"),
            Self::OtherBookmarks => write!(f, "Other Bookmarks"),
            Self::MobileBookmarks => write!(f, "Mobile Bookmarks"),
            Self::BookmarkMenu => write!(f, "Bookmark Menu"),
            Self::Toolbar => write!(f, "Toolbar Bookmarks"),
            Self::Unfiled => write!(f, "Unfiled Bookmarks"),
            Self::ReadingList => write!(f, "Reading List"),
            Self::Favorites => write!(f, "Favorites"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// Folder representation preserving lineage and position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderNode {
    pub id: String,
    pub title: String,
    pub children: Vec<BookmarkNode>,
    pub source: SourceDescriptor,
    pub root_kind: BookmarkRootKind,
    pub original_position: usize,
    pub date_added: Option<DateTime<Utc>>,
    pub date_modified: Option<DateTime<Utc>>,
}

/// Bookmark occurrence preserving exact raw URL and audit parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkOccurrence {
    pub id: String,
    pub title: String,
    pub raw_url: String,
    pub source: SourceDescriptor,
    pub root_kind: BookmarkRootKind,
    pub folder_path: Vec<String>,
    pub original_position: usize,
    pub date_added: Option<DateTime<Utc>>,
    pub date_modified: Option<DateTime<Utc>>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

/// Separator node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeparatorNode {
    pub id: String,
    pub source: SourceDescriptor,
    pub original_position: usize,
}

/// Generalized node in internal bookmark tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BookmarkNode {
    Folder(FolderNode),
    Bookmark(BookmarkOccurrence),
    Separator(SeparatorNode),
}

impl BookmarkNode {
    pub const fn is_folder(&self) -> bool {
        matches!(self, Self::Folder(_))
    }

    pub const fn is_bookmark(&self) -> bool {
        matches!(self, Self::Bookmark(_))
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Folder(f) => &f.title,
            Self::Bookmark(b) => &b.title,
            Self::Separator(_) => "---",
        }
    }
}

/// Top-level bookmark tree containing complete harvested items
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkTree {
    pub sources: Vec<SourceDescriptor>,
    pub roots: Vec<BookmarkNode>,
    pub harvested_at: DateTime<Utc>,
}
