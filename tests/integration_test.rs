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
fn test_migrate_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["migrate", "--dry-run"]);

    // Should identify hub browsers
    assert!(
        stderr.contains("Hub") || stdout.contains("Hub"),
        "Should identify hub browsers"
    );

    // Should show dry run summary
    assert!(
        stderr.contains("Dry run") || stdout.contains("Dry run"),
        "Should indicate dry run mode"
    );

    println!("✅ migrate --dry-run works");
}

#[test]
fn test_migrate_with_history_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["migrate", "--history", "--dry-run"]);

    // Should show history sync
    assert!(
        stderr.contains("history") || stdout.contains("history"),
        "Should mention history synchronization"
    );

    // Should show merged result
    assert!(
        stderr.contains("Merged") || stderr.contains("items") || stdout.contains("Merged") || stdout.contains("items"),
        "Should show merge results"
    );

    println!("✅ migrate --history --dry-run works");
}

#[test]
fn test_migrate_with_clear_others_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["migrate", "--clear-others", "--dry-run"]);

    // Should show non-hub browsers will be cleared
    assert!(
        stderr.contains("cleared") || stderr.contains("Non-hub") || 
        stdout.contains("cleared") || stdout.contains("Non-hub"),
        "Should indicate non-hub browsers will be cleared"
    );

    println!("✅ migrate --clear-others --dry-run works");
}

#[test]
fn test_help_commands() {
    // Test main help - help goes to stdout
    let (_, stdout, stderr) = run_cli(&["--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("migrate") || combined.contains("Commands"),
        "Help should list available commands"
    );

    // Test subcommand help
    let (_, stdout, stderr) = run_cli(&["migrate", "--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("browsers") || combined.contains("hub") || combined.contains("Hub"),
        "migrate help should show options"
    );

    println!("✅ help commands work");
}

#[test]
fn test_full_migration_dry_run() {
    // Test full migration with all options
    let (_success, stdout, stderr) = run_cli(&[
        "migrate",
        "--browsers", "waterfox,brave-nightly",
        "--history",
        "--clear-others",
        "--dry-run",
        "--verbose"
    ]);

    let combined = format!("{}{}", stdout, stderr);
    
    // Should show hub browsers
    assert!(combined.contains("waterfox") || combined.contains("Waterfox"), 
        "Should mention Waterfox");
    assert!(combined.contains("brave") || combined.contains("Brave"), 
        "Should mention Brave");
    
    // Should show data statistics
    assert!(combined.contains("URLs") || combined.contains("bookmarks"), 
        "Should show bookmark statistics");
    
    // Should complete successfully
    assert!(combined.contains("Migration complete") || combined.contains("Summary"), 
        "Should complete migration");

    println!("✅ full migration dry-run works");
}
