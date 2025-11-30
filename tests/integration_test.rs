use browser_bookmark_sync::browsers::*;
use browser_bookmark_sync::sync::SyncEngine;

#[test]
fn test_bookmark_structure() {
    let bookmark = Bookmark {
        id: "test-1".to_string(),
        title: "Test Bookmark".to_string(),
        url: Some("https://example.com".to_string()),
        folder: false,
        children: vec![],
        date_added: Some(1234567890),
        date_modified: Some(1234567900),
    };
    
    assert_eq!(bookmark.title, "Test Bookmark");
    assert_eq!(bookmark.url, Some("https://example.com".to_string()));
    assert!(!bookmark.folder);
}

#[test]
fn test_browser_types() {
    assert_eq!(BrowserType::Safari.name(), "Safari");
    assert_eq!(BrowserType::Brave.name(), "Brave");
    assert_eq!(BrowserType::Waterfox.name(), "Waterfox");
    assert_eq!(BrowserType::Nightly.name(), "Firefox Nightly");
}

#[tokio::test]
async fn test_sync_engine_creation() {
    let result = SyncEngine::new();
    assert!(result.is_ok(), "SyncEngine should be created successfully");
}
