#[cfg(unix)]
#[test]
fn sigterm_marks_check_aborted_and_logs_reason() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        fs,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    let status_path = temp.path().join("check-status.json");
    let log_path = temp.path().join("browser-sync.log");
    let db_path = temp.path().join("bookmark-health.sqlite");

    let mut html = String::from("<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p>\n");
    for index in 0..100 {
        html.push_str(&format!(
            "<DT><A HREF=\"https://example.com/signal-test-{index}\">test</A>\n"
        ));
    }
    html.push_str("</DL><p>\n");
    fs::write(&input, html)?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"))
        .args([
            "--log-file",
            &log_path.to_string_lossy(),
            "check",
            "--file",
            &input.to_string_lossy(),
            "--history-db",
            &db_path.to_string_lossy(),
            "--export-dir",
            &temp.path().to_string_lossy(),
            "--scope",
            "content",
            "--timeout",
            "10",
            "--concurrency",
            "1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(10);
    while !status_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    if !status_path.exists() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("check process did not create status file".into());
    }

    let signal_status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    if !signal_status.success() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("kill command failed".into());
    }
    let exit_status = child.wait()?;
    assert!(
        !exit_status.success(),
        "SIGTERM should stop the check process"
    );

    let status: serde_json::Value = serde_json::from_str(&fs::read_to_string(&status_path)?)?;
    assert_eq!(status["status"], "aborted");

    let log = fs::read_to_string(&log_path)?;
    assert!(log.contains("process_terminated"));
    assert!(log.contains("check_status_aborted"));
    assert!(log.contains("SIGTERM"));
    Ok(())
}
