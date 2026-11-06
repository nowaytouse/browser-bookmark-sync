// Integration tests for browser-bookmark-sync
// Run with: cargo test --test integration_test

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::nursery
)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SAFARI_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>WebBookmarkType</key><string>WebBookmarkTypeList</string>
<key>Children</key><array><dict><key>WebBookmarkType</key><string>WebBookmarkTypeLeaf</string>
<key>URLString</key><string>https://fixture.example.test/one</string>
<key>URIDictionary</key><dict><key>title</key><string>Synthetic bookmark</string></dict>
</dict></array></dict></plist>"#;

fn run_cli(args: &[&str]) -> (bool, String, String) {
    // Child-only fixture roots: ordinary tests must not inspect the caller's Profiles.
    let fixture = tempfile::tempdir().expect("create isolated CLI fixture");
    let safari = fixture.path().join("Library/Safari/Bookmarks.plist");
    std::fs::create_dir_all(safari.parent().unwrap()).unwrap();
    std::fs::write(&safari, SAFARI_FIXTURE).unwrap();
    let bin_path = env!("CARGO_BIN_EXE_browser-bookmark-sync");
    let mut child = Command::new(bin_path)
        .args(args)
        .current_dir(fixture.path())
        .env("HOME", fixture.path())
        .env("USERPROFILE", fixture.path())
        .env("APPDATA", fixture.path())
        .env("LOCALAPPDATA", fixture.path())
        .env("XDG_CONFIG_HOME", fixture.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to execute command");
    let started = Instant::now();
    loop {
        if child.try_wait().expect("poll CLI fixture").is_some() {
            break;
        }
        if started.elapsed() >= Duration::from_secs(10) {
            child.kill().expect("kill timed-out CLI fixture");
            child.wait().expect("reap timed-out CLI fixture");
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "CLI fixture exceeded 10 seconds: {args:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("read CLI fixture output");
    assert_eq!(std::fs::read(&safari).unwrap(), SAFARI_FIXTURE.as_bytes());

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

#[test]
fn test_list_command() {
    let (success, stdout, stderr) = run_cli(&["list"]);
    assert!(success, "{stderr}");

    // Listing may inspect only the private fixture roots.
    assert!(
        stderr.contains("Detected Browsers") || stdout.contains("Detected Browsers"),
        "Should show detected browsers section"
    );

    // On macOS, the synthetic Safari source should be detected.
    #[cfg(target_os = "macos")]
    assert!(
        stderr.contains("Safari") || stdout.contains("Safari"),
        "synthetic Safari should be detected on macOS"
    );

    println!("✅ list command works");
}

#[test]
fn test_validate_command() {
    let (success, stdout, stderr) = run_cli(&["validate"]);
    assert!(success, "{stderr}");

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
    let (success, stdout, stderr) = run_cli(&["history", "--dry-run"]);
    assert!(
        !success,
        "non-bookmark history commands must remain disabled"
    );

    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("disabled"),
        "Should explain that history synchronization is disabled"
    );

    println!("disabled history command is rejected");
}

#[test]
fn test_analyze_command() {
    let (success, stdout, stderr) = run_cli(&["analyze"]);
    assert!(success, "{stderr}");
    #[cfg(target_os = "macos")]
    assert!(
        stdout.contains("Total bookmarks: 1"),
        "analysis must read the synthetic fixture: {stdout}"
    );

    let combined = format!("{stdout}{stderr}");
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

    let combined = format!("{stdout}{stderr}");
    assert!(
        success
            && (combined.contains("rule")
                || combined.contains("Rule")
                || combined.contains("classification")),
        "Should show classification rules"
    );

    println!("✅ rules command works");
}

#[test]
fn test_help_commands() {
    // Test main help
    let (_, stdout, stderr) = run_cli(&["--help"]);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("export") && combined.contains("validate"),
        "Help should list available commands"
    );

    // Test export help
    let (_, stdout, stderr) = run_cli(&["export", "--help"]);
    let combined = format!("{stdout}{stderr}");
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
    let combined = format!("{stdout}{stderr}");

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
