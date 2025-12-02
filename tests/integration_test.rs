// Integration tests for browser-bookmark-sync
// Run with: cargo test --test integration_test

use std::process::Command;

fn run_cli(args: &[&str]) -> (bool, String, String) {
    let output = Command::new("cargo")
        .args(["run", "--release", "--"])
        .args(args)
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

#[test]
fn test_list_command() {
    let (_success, stdout, stderr) = run_cli(&["list"]);

    // Should detect at least some browsers
    assert!(
        stderr.contains("Detected Browsers") || stdout.contains("Detected Browsers"),
        "Should show detected browsers section"
    );

    // On macOS, Safari should always be detected
    #[cfg(target_os = "macos")]
    assert!(
        stderr.contains("Safari") || stdout.contains("Safari"),
        "Safari should be detected on macOS"
    );

    println!("✅ list command works");
}

#[test]
fn test_validate_command() {
    let (_success, stdout, stderr) = run_cli(&["validate"]);

    // Should show validation report
    assert!(
        stderr.contains("Validation Report") || stdout.contains("Validation Report"),
        "Should show validation report"
    );

    // Should show summary
    assert!(
        stderr.contains("Summary") || stdout.contains("Summary"),
        "Should show summary"
    );

    println!("✅ validate command works");
}

#[test]
fn test_sync_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["sync", "--dry-run"]);

    // Should show full sync (bookmarks + history + cookies)
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("bookmarks") || combined.contains("Merged"),
        "Should show bookmark sync"
    );
    assert!(
        combined.contains("history") || combined.contains("History"),
        "Should show history sync"
    );
    assert!(
        combined.contains("cookies") || combined.contains("Cookies"),
        "Should show cookies sync"
    );
    assert!(
        combined.contains("Dry run") || combined.contains("dry run"),
        "Should indicate dry run mode"
    );

    println!("✅ sync --dry-run works (full sync: bookmarks + history + cookies)");
}

#[test]
fn test_sync_history_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["sync-history", "--dry-run"]);

    // Should sync ALL history (no --days option)
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("history") || combined.contains("History"),
        "Should mention history synchronization"
    );
    assert!(
        combined.contains("Merged") || combined.contains("unique"),
        "Should show merge results"
    );

    println!("✅ sync-history --dry-run works (syncs ALL history)");
}

#[test]
fn test_sync_with_custom_browsers() {
    let (_success, stdout, stderr) =
        run_cli(&["sync", "--browsers", "waterfox,brave-nightly", "--dry-run"]);

    let combined = format!("{}{}", stdout, stderr);
    // Should identify hub browsers
    assert!(
        combined.contains("Hub") || combined.contains("hub") || combined.contains("waterfox"),
        "Should identify hub browsers"
    );

    println!("✅ sync --browsers works");
}

#[test]
fn test_sync_clear_others_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["sync", "--clear-others", "--dry-run"]);

    let combined = format!("{}{}", stdout, stderr);
    // Should show that non-hub browsers will be cleared
    assert!(
        combined.contains("Non-hub") || combined.contains("clear") || combined.contains("CLEARED"),
        "Should indicate non-hub browsers will be cleared"
    );

    println!("✅ sync --clear-others --dry-run works");
}

#[test]
fn test_help_commands() {
    // Test main help
    let (_, stdout, stderr) = run_cli(&["--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("sync") && combined.contains("validate"),
        "Help should list available commands"
    );

    // Test sync help - should show full sync description
    let (_, stdout, stderr) = run_cli(&["sync", "--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("bookmarks") || combined.contains("history") || combined.contains("Full"),
        "sync help should mention full sync"
    );

    println!("✅ help commands work");
}

#[test]
fn test_sync_cookies_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["sync-cookies", "--dry-run"]);

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("cookies") || combined.contains("Cookies"),
        "Should mention cookies synchronization"
    );

    println!("✅ sync-cookies --dry-run works");
}
