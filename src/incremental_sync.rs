use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::browsers::BrowserType;

/// Sync state for incremental sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub last_sync_time: DateTime<Utc>,
    pub browser_states: HashMap<BrowserType, BrowserSyncState>,
}

/// Per-browser sync state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSyncState {
    pub bookmark_hashes: HashMap<String, String>, // URL -> hash(url+title+folder_path)
    pub last_modified: DateTime<Utc>,
}

impl SyncState {
    /// Create new sync state
    pub fn new() -> Self {
        Self {
            last_sync_time: Utc::now(),
            browser_states: HashMap::new(),
        }
    }
    
    /// Load sync state from file
    pub fn load() -> Result<Self> {
        let path = Self::get_state_path()?;
        
        if !path.exists() {
            return Ok(Self::new());
        }
        
        let content = fs::read_to_string(&path)?;
        let state: SyncState = serde_json::from_str(&content)?;
        
        Ok(state)
    }
    
    /// Save sync state to file
    pub fn save(&self) -> Result<()> {
        let path = Self::get_state_path()?;
        
        // Create parent directory if not exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let content = serde_json::to_string_pretty(&self)?;
        fs::write(&path, content)?;
        
        Ok(())
    }
    
    /// Get state file path
    fn get_state_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home).join(".browser-sync-state.json"))
    }
    
    /// Update browser state
    pub fn update_browser_state(&mut self, browser: BrowserType, state: BrowserSyncState) {
        self.browser_states.insert(browser, state);
        self.last_sync_time = Utc::now();
    }
}

impl Default for SyncState {
    fn default() -> Self {
        Self::new()
    }
}

/// Bookmark change type
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
}

/// Bookmark change
#[derive(Debug, Clone)]
pub struct BookmarkChange {
    pub url: String,
    pub title: Option<String>,
    pub folder_path: Option<String>,
    pub change_type: ChangeType,
    pub timestamp: DateTime<Utc>,
    pub source_browser: BrowserType,
}

/// Detect changes between current bookmarks and last sync state
pub fn detect_changes(
    current_bookmarks: &[(String, String, String)], // (url, title, folder_path)
    last_state: &HashMap<String, String>, // URL -> hash
    browser: BrowserType,
) -> Vec<BookmarkChange> {
    let mut changes = Vec::new();
    let now = Utc::now();
    
    // Build current hash map
    let mut current_hashes: HashMap<String, String> = HashMap::new();
    for (url, title, folder_path) in current_bookmarks {
        let hash = compute_bookmark_hash(url, title, folder_path);
        current_hashes.insert(url.clone(), hash);
    }
    
    // Detect added and modified
    for (url, title, folder_path) in current_bookmarks {
        let current_hash = compute_bookmark_hash(url, title, folder_path);
        
        match last_state.get(url) {
            None => {
                // New bookmark
                changes.push(BookmarkChange {
                    url: url.clone(),
                    title: Some(title.clone()),
                    folder_path: Some(folder_path.clone()),
                    change_type: ChangeType::Added,
                    timestamp: now,
                    source_browser: browser,
                });
            }
            Some(last_hash) if last_hash != &current_hash => {
                // Modified bookmark
                changes.push(BookmarkChange {
                    url: url.clone(),
                    title: Some(title.clone()),
                    folder_path: Some(folder_path.clone()),
                    change_type: ChangeType::Modified,
                    timestamp: now,
                    source_browser: browser,
                });
            }
            _ => {
                // Unchanged
            }
        }
    }
    
    // Detect deleted
    for (url, _) in last_state {
        if !current_hashes.contains_key(url) {
            changes.push(BookmarkChange {
                url: url.clone(),
                title: None,
                folder_path: None,
                change_type: ChangeType::Deleted,
                timestamp: now,
                source_browser: browser,
            });
        }
    }
    
    changes
}

/// Compute hash for bookmark
fn compute_bookmark_hash(url: &str, title: &str, folder_path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    title.hash(&mut hasher);
    folder_path.hash(&mut hasher);
    
    format!("{:x}", hasher.finish())
}

/// Merge changes from multiple browsers
pub fn merge_changes(
    all_changes: Vec<Vec<BookmarkChange>>,
) -> Vec<BookmarkChange> {
    let mut merged: HashMap<String, BookmarkChange> = HashMap::new();
    
    // Flatten all changes
    for changes in all_changes {
        for change in changes {
            let url = change.url.clone();
            
            // Conflict resolution: latest timestamp wins
            match merged.get(&url) {
                None => {
                    merged.insert(url, change);
                }
                Some(existing) => {
                    if change.timestamp > existing.timestamp {
                        merged.insert(url, change);
                    }
                    // If timestamps equal, keep the first one (arbitrary but consistent)
                }
            }
        }
    }
    
    merged.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_detect_added() {
        let current = vec![
            ("https://example.com".to_string(), "Example".to_string(), "Folder1".to_string()),
        ];
        let last_state = HashMap::new();
        
        let changes = detect_changes(&current, &last_state, BrowserType::Safari);
        
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, ChangeType::Added);
    }
    
    #[test]
    fn test_detect_deleted() {
        let current = vec![];
        let mut last_state = HashMap::new();
        last_state.insert(
            "https://example.com".to_string(),
            compute_bookmark_hash("https://example.com", "Example", "Folder1"),
        );
        
        let changes = detect_changes(&current, &last_state, BrowserType::Safari);
        
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, ChangeType::Deleted);
    }
    
    #[test]
    fn test_merge_conflict_latest_wins() {
        use chrono::Duration;
        
        let now = Utc::now();
        let earlier = now - Duration::seconds(10);
        
        let change1 = BookmarkChange {
            url: "https://example.com".to_string(),
            title: Some("Old Title".to_string()),
            folder_path: Some("Folder1".to_string()),
            change_type: ChangeType::Modified,
            timestamp: earlier,
            source_browser: BrowserType::Safari,
        };
        
        let change2 = BookmarkChange {
            url: "https://example.com".to_string(),
            title: Some("New Title".to_string()),
            folder_path: Some("Folder2".to_string()),
            change_type: ChangeType::Modified,
            timestamp: now,
            source_browser: BrowserType::Chrome,
        };
        
        let merged = merge_changes(vec![vec![change1], vec![change2]]);
        
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title.as_ref().unwrap(), "New Title");
    }
}
