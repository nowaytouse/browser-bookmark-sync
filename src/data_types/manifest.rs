//! Run Manifest and Inventory Audit Models

use crate::data_types::models::{BrowserInstallation, BrowserProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Inventory audit report summarizing all discovered installations and profiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryReport {
    pub schema_version: String,
    pub generated_at: DateTime<Utc>,
    pub total_installations: usize,
    pub total_profiles: usize,
    pub total_bookmarks: usize,
    pub warnings: Vec<String>,
    pub installations: Vec<BrowserInstallationSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserInstallationSummary {
    pub installation: BrowserInstallation,
    pub profiles: Vec<BrowserProfileSummary>,
    /// Error encountered while enumerating this installation, if any.
    #[serde(default)]
    pub discovery_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfileSummary {
    pub profile: BrowserProfile,
    pub bookmark_count: usize,
    pub folder_count: usize,
    pub snapshot_path: Option<PathBuf>,
    /// Whether the profile was snapshotted and parsed successfully.
    #[serde(default)]
    pub read_success: bool,
    /// A reportable reason when the profile could not be read.
    #[serde(default)]
    pub error: Option<String>,
}

/// Execution manifest capturing parameters, hashes, output paths, and invariants
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema_version: String,
    pub run_id: String,
    pub timestamp: DateTime<Utc>,
    pub options: BTreeMap<String, String>,
    pub total_sources: usize,
    pub total_raw_occurrences: usize,
    pub total_canonical_bookmarks: usize,
    pub total_removed_duplicates: usize,
    pub invariant_satisfied: bool,
    pub warnings: Vec<String>,
    pub output_files: BTreeMap<String, String>,
    pub blake3_snapshot_hashes: BTreeMap<String, String>,
}
