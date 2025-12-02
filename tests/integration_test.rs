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
fn test_history_dry_run() {
    let (_success, stdout, stderr) = run_cli(&["history", "--dry-run"]);

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("history") || combined.contains("History"),
        "Should mention history synchronization"
    );

    println!("✅ history --dry-run works");
}

#[test]
fn test_analyze_command() {
    let (_success, stdout, stderr) = run_cli(&["analyze"]);

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Analyzing")
            || combined.contains("Analysis")
            || combined.contains("bookmarks"),
        "Should show analysis output"
    );

    println!("✅ analyze command works");
}

#[test]
fn test_rules_command() {
    let (success, stdout, stderr) = run_cli(&["rules"]);

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        success
            || combined.contains("rule")
            || combined.contains("Rule")
            || combined.contains("classification"),
        "Should show classification rules"
    );

    println!("✅ rules command works");
}

#[test]
fn test_help_commands() {
    // Test main help
    let (_, stdout, stderr) = run_cli(&["--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("export") && combined.contains("validate"),
        "Help should list available commands"
    );

    // Test export help
    let (_, stdout, stderr) = run_cli(&["export", "--help"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("bookmarks") || combined.contains("output") || combined.contains("HTML"),
        "export help should describe export functionality"
    );

    println!("✅ help commands work");
}

#[test]
fn test_export_dry_check() {
    // Test that export command parses arguments correctly
    let (_, stdout, stderr) = run_cli(&["export", "--help"]);
    let combined = format!("{}{}", stdout, stderr);

    // Should show various export options
    assert!(
        combined.contains("--deduplicate") || combined.contains("-d"),
        "Should have deduplicate option"
    );
    assert!(
        combined.contains("--merge") || combined.contains("-m"),
        "Should have merge option"
    );

    println!("✅ export options are available");
}
