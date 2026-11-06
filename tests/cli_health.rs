use std::{
    error::Error,
    io,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use rusqlite::Connection;
use tempfile::TempDir;
use tokio::{process::Command, time::timeout};

const CLI: &str = env!("CARGO_BIN_EXE_browser-bookmark-sync");

async fn wait_for_exit(
    args: &[String],
    duration: Duration,
) -> Result<Option<ExitStatus>, Box<dyn Error>> {
    let mut child = Command::new(CLI)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    match timeout(duration, child.wait()).await {
        Ok(Ok(status)) => Ok(Some(status)),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error.into())
        }
        Err(_) => {
            let _ = child.kill().await;
            child.wait().await?;
            Ok(None)
        }
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

fn db_arg_args(command: &str, db: &std::path::Path) -> Vec<String> {
    strings(&["health", command, "--db"])
        .into_iter()
        .chain(std::iter::once(db.to_string_lossy().into_owned()))
        .collect()
}

fn initialize_db(temp: &TempDir) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let db = temp.path().join("health.sqlite");
    browser_bookmark_sync::health::db::HealthDatabase::open(&db)?;
    Ok(db)
}

#[tokio::test]
async fn watch_and_monitor_exit_nonzero_on_database_errors() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let invalid_db = temp.path().join("not-sqlite.sqlite");
    std::fs::write(&invalid_db, "not a SQLite database")?;

    for args in [
        {
            let mut args = strings(&["health", "summary", "--watch", "--db"]);
            args.push(invalid_db.to_string_lossy().into_owned());
            args
        },
        {
            let mut args = strings(&["health", "monitor", "--db"]);
            args.push(invalid_db.to_string_lossy().into_owned());
            args
        },
    ] {
        let status = wait_for_exit(&args, Duration::from_secs(3)).await?;
        assert!(
            status.is_some_and(|status| !status.success()),
            "watch/monitor must exit nonzero on DB errors; args={args:?}, status={status:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn retry_quarantine_and_rebuild_summary_fail_for_missing_database(
) -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let missing_db = temp.path().join("missing.sqlite");

    for command in ["retry", "quarantine", "rebuild-summary"] {
        let mut args = db_arg_args(command, &missing_db);
        if command == "quarantine" {
            args.push("--output".to_string());
            args.push(
                temp.path()
                    .join("selection.json")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let status = wait_for_exit(&args, Duration::from_secs(5)).await?;
        assert!(
            status.is_some_and(|status| !status.success()),
            "{command} must fail when its requested DB is missing; status={status:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn retry_rejects_all_rows_if_any_target_url_is_invalid() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = initialize_db(&temp)?;
    let conn = Connection::open(&db)?;
    conn.execute(
        "INSERT INTO health_summaries (summary_id, bookmark_id, url, last_run_id, last_checked_at, current_verdict, total_checks, consecutive_failures) \
         VALUES ('summary-invalid', 'invalid-bookmark', 'https://co.uk/', 'fixture-run', '2026-10-17T00:00:00Z', 'needs_review', 1, 1)",
        [],
    )
    ?;

    let mut args = db_arg_args("retry", &db);
    args.extend(strings(&["--state", "needs_review"]));
    let status = wait_for_exit(&args, Duration::from_secs(5)).await?;
    assert!(
        status.is_some_and(|status| !status.success()),
        "retry must fail before probing when any selected URL cannot become a ProbeTarget; status={status:?}"
    );
    Ok(())
}

#[tokio::test]
async fn retry_with_no_matching_rows_succeeds_without_probing() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = initialize_db(&temp)?;
    let args = db_arg_args("retry", &db);
    let status = wait_for_exit(&args, Duration::from_secs(5)).await?;
    assert!(
        status.is_some_and(|status| status.success()),
        "an existing DB with no matching rows remains a successful no-op; status={status:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn monitor_waiting_for_missing_database_exits_on_sigterm() -> Result<(), Box<dyn Error>> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let temp = tempfile::tempdir()?;
    let missing_db = temp.path().join("not-created-yet.sqlite");
    let missing_db = missing_db
        .to_str()
        .ok_or_else(|| io::Error::other("temporary DB path is not UTF-8"))?;
    let mut child = Command::new(CLI)
        .args(["health", "monitor", "--db", missing_db, "--interval", "60"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(io::Error::other("monitor stdout was not captured").into());
        }
    };
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let startup = timeout(Duration::from_secs(5), async {
        loop {
            line.clear();
            let count = reader.read_line(&mut line).await?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "monitor closed stdout before entering its missing-DB wait state",
                ));
            }
            if line.contains("尚未创建") {
                return Ok(());
            }
        }
    })
    .await;
    match startup {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.into());
        }
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.into());
        }
    }

    let pid = match child.id() {
        Some(pid) => pid,
        None => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(io::Error::other("monitor has no process ID").into());
        }
    };
    match std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
    {
        Ok(signal) if signal.success() => {}
        Ok(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(io::Error::other("kill command failed").into());
        }
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.into());
        }
    }

    match timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => assert!(!status.success(), "SIGTERM should fail the CLI run"),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.into());
        }
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.into());
        }
    }
    Ok(())
}
