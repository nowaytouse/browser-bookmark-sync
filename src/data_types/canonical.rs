//! Canonical Bookmark Models and Deduplication Reporting Structs

use crate::data_types::models::BookmarkOccurrence;
use serde::{Deserialize, Serialize};

/// Reasons why a specific occurrence was selected as primary
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionReason {
    PreferredSource,
    PreferredRootKind,
    DeeperFolderPath,
    EarliestDateAdded,
    DeterministicFallback,
}

impl std::fmt::Display for SelectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreferredSource => write!(f, "preferred_source"),
            Self::PreferredRootKind => write!(f, "preferred_root_kind"),
            Self::DeeperFolderPath => write!(f, "deeper_folder_path"),
            Self::EarliestDateAdded => write!(f, "earliest_date_added"),
            Self::DeterministicFallback => write!(f, "deterministic_fallback"),
        }
    }
}

/// Logical canonical bookmark grouping a primary occurrence with all duplicates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalBookmark {
    pub canonical_id: String,
    pub exact_url: String,
    pub primary: BookmarkOccurrence,
    pub duplicates: Vec<BookmarkOccurrence>,
    pub selection_reason: Vec<SelectionReason>,
}

/// Audit report group for serialization to JSON/CSV
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroupReport {
    pub group_id: String,
    pub exact_url: String,
    pub primary_title: String,
    pub primary_source: String,
    pub primary_folder_path: String,
    pub duplicate_count: usize,
    pub duplicates: Vec<BookmarkOccurrence>,
    pub selection_reasons: Vec<String>,
}

/// Flat CSV record for audit logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateCsvRecord {
    pub group_id: String,
    pub exact_url: String,
    pub action: String,
    pub browser: String,
    pub profile: String,
    pub root: String,
    pub folder_path: String,
    pub title: String,
    pub date_added: String,
    pub date_modified: String,
    pub original_position: usize,
    pub selection_reason: String,
}
