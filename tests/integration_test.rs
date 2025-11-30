// Integration tests for browser-bookmark-sync
// Run with: cargo test --test integration_test

use std::process::Command;
use std::path::Path;

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
    let (success, stdout, stderr) = run_cli(&["list"]);
    
    // Should detect at least some browsers
    assert!(stderr.contains("Detected Browsers") || stdout.contains("Detected Browsers"), 
        "Should show detected browsers section");
    
    // On macOS, Safari should always be detected
    #[cfg(target_os = "macos")]
    assert!(stderr.contains("Safari") || stdout.contains("Safari"), 
        "Safari should be detected on macOS");
    
    println!("✅ list command works");
}

#[test]
fn test_validate_command() {
    let (success, stdout, stderr) = run_cli(&["validate"]);
    
    // Should show validation report
    assert!(stderr.contains("Validation Report") || stdout.contains("Validation Report"),
        "Should show validation report");
    
    // Should show summary
    assert!(stderr.contains("Summary") || stdout.contains("Summary"),
        "Should show summary");
    
    println!("✅ validate command works");
}

#[test]
fn test_sync_dry_run() {
    let (success, stdout, stderr) = run_cli(&["sync", "--dry-run"]);
    
    // Dry run should not fail
    assert!(stderr.contains("Dry run") || stdout.contains("Dry run"),
        "Should indicate dry run mode");
    
    println!("✅ sync --dry-run works");
}

#[test]
fn test_sync_history_dry_run() {
    let (success, stdout, stderr) = run_cli(&["sync-history", "--dry-run", "--days", "7"]);
    
    // Should show history sync phases
    assert!(stderr.contains("history") || stdout.contains("history"),
        "Should mention history synchronization");
    
    // Should show merged result
    assert!(stderr.contains("Merged") || stderr.contains("unique") || 
            stdout.contains("Merged") || stdout.contains("unique"),
        "Should show merge results");
    
    println!("✅ sync-history --dry-run works");
}

#[test]
fn test_set_hubs_dry_run() {
    let (success, stdout, stderr) = run_cli(&[
        "set-hubs",
        "--browsers", "waterfox,brave-nightly",
        "--dry-run"
    ]);
    
    // Should identify hub browsers
    assert!(stderr.contains("Hub") || stdout.contains("Hub"),
        "Should identify hub browsers");
    
    // Should show dry run summary
    assert!(stderr.contains("Dry run") || stdout.contains("Dry run"),
        "Should indicate dry run mode");
    
    println!("✅ set-hubs --dry-run works");
}

#[test]
fn test_help_commands() {
    // Test main help - help goes to stdout
    let (_, stdout, stderr) = run_cli(&["--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(combined.contains("sync") || combined.contains("Commands"),
        "Help should list available commands");
    
    // Test subcommand help
    let (_, stdout, stderr) = run_cli(&["set-hubs", "--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(combined.contains("browsers") || combined.contains("hub") || combined.contains("Hub"),
        "set-hubs help should show options");
    
    println!("✅ help commands work");
}
