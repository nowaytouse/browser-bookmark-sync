//! User-started checks reuse the existing independent supervisor.

use crate::native_preferences::NetworkSettings;
use anyhow::{ensure, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
#[cfg(target_os = "macos")]
use std::fs::OpenOptions;
#[cfg(target_os = "macos")]
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Clone)]
pub struct ScanHandle {
    pub directory: PathBuf,
}

#[derive(Default)]
pub struct ScanStatus {
    pub state: String,
    pub phase: String,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub exit_code: Option<i64>,
}

impl ScanHandle {
    pub fn input(&self) -> PathBuf {
        self.directory.join("input.html")
    }
    pub fn db(&self) -> PathBuf {
        self.directory.join("bookmark-health.sqlite")
    }
    pub fn report(&self) -> PathBuf {
        self.directory.join("import-report.json")
    }

    pub fn status(&self) -> Result<ScanStatus> {
        let status = read_optional(&self.directory.join("health/check-status.json"))?;
        let receipt = read_optional(&self.directory.join("process-exit.json"))?;
        let manifest = read_optional(&self.directory.join("run-manifest.json"))?;
        let exit_code = receipt
            .as_ref()
            .and_then(|v| v["process_exit_code"].as_i64());
        Ok(ScanStatus {
            state: if exit_code.is_some_and(|code| code != 0) {
                "failed"
            } else {
                status
                    .as_ref()
                    .and_then(|v| v["status"].as_str())
                    .or_else(|| manifest.as_ref().and_then(|v| v["status"].as_str()))
                    .unwrap_or("starting")
            }
            .into(),
            phase: status
                .as_ref()
                .and_then(|v| v["phase"].as_str())
                .unwrap_or("preparing")
                .into(),
            completed: status
                .as_ref()
                .and_then(|v| v["completed_targets"].as_u64()),
            total: status.as_ref().and_then(|v| v["total_targets"].as_u64()),
            exit_code,
        })
    }

    /// Verify every eligible Site/Content target before presenting a complete run.
    pub fn verify_complete(&self) -> Result<()> {
        self.verify_input()?;
        let status = self.status()?;
        ensure!(
            status.exit_code == Some(0),
            "No successful process exit receipt"
        );
        let plan: Value =
            serde_json::from_slice(&fs::read(self.directory.join("health/probe-plan.json"))?)?;
        ensure!(
            plan["scope"] == "content" && plan["partial"] == false,
            "The plan is not a full content check"
        );
        let targets = plan["targets"]
            .as_array()
            .context("Content target list missing")?;
        ensure!(status.state == "completed", "Checking is not complete");
        if !targets.is_empty() {
            ensure!(
                status.phase == "content"
                    && status.total.is_some()
                    && status.completed == status.total,
                "Content coverage counters are incomplete"
            );
        }
        let conn = Connection::open_with_flags(self.db(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.pragma_update(None, "query_only", "ON")?;
        let tx = conn.unchecked_transaction()?;
        let mut query = tx.prepare("SELECT DISTINCT c.original_url FROM health_checks c JOIN health_runs r ON r.run_id=c.run_id WHERE r.completed=1 AND c.finished_at IS NOT NULL AND (c.bookmark_id LIKE 'site:%')=?1")?;
        for (site, targets) in [
            (false, &plan["targets"]),
            (true, &plan["site_preflight"]["targets"]),
        ] {
            let observed = query
                .query_map([site], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()?;
            let targets = targets
                .as_array()
                .context("Probe plan target list missing")?;
            for target in targets {
                let url = target["probe_url"].as_str().context("Probe URL missing")?;
                ensure!(
                    observed.contains(url),
                    "Missing completed evidence for a planned target"
                );
            }
        }
        Ok(())
    }

    fn verify_input(&self) -> Result<()> {
        #[cfg(not(target_os = "macos"))]
        anyhow::bail!("Independent batch verification currently requires macOS");
        #[cfg(target_os = "macos")]
        {
            let manifest = read_optional(&self.directory.join("run-manifest.json"))?
                .context("Launch manifest missing")?;
            let expected = manifest["input_sha256"]
                .as_str()
                .context("Launch input hash missing")?;
            ensure!(
                sha256(&self.input())? == expected,
                "Batch input changed since launch"
            );
            Ok(())
        }
    }
}

fn read_optional(path: &Path) -> Result<Option<Value>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).with_context(|| format!("Read {}", path.display()))?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn begin(
    input: &Path,
    expected_hash: &str,
    report: Option<&Value>,
    network: NetworkSettings,
) -> Result<ScanHandle> {
    network.validate().map_err(anyhow::Error::msg)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (input, expected_hash, report, network);
        anyhow::bail!("Independent checks currently require macOS");
    }
    #[cfg(target_os = "macos")]
    {
        let preferences = crate::native_preferences::PreferencesStore::local()?;
        let root = preferences
            .path()
            .parent()
            .context("App data directory missing")?
            .join("checks");
        fs::create_dir_all(&root)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let directory = tempfile::Builder::new()
            .prefix("clean-")
            .tempdir_in(&root)?
            .keep();
        let handle = ScanHandle { directory };
        let current = std::env::current_exe()?;
        let parent = current.parent().context("Executable directory missing")?;
        let binary = [
            parent.join("bsync"),
            parent.join("browser-bookmark-sync"),
            parent.join("../Resources/bsync"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .context("The App's check engine is missing. Reinstall the complete App bundle.")?;
        let bytes = fs::read(input)?;
        ensure!(
            blake3::hash(&bytes).to_hex().as_str() == expected_hash,
            "Input changed since import"
        );
        write_new(&handle.input(), &bytes)?;
        fs::copy(binary, handle.directory.join("bsync"))?;
        if let Some(report) = report {
            write_new(&handle.report(), &serde_json::to_vec_pretty(report)?)?;
        }
        let dns_policy = handle.directory.join("dns-policy.json");
        write_new(
            &dns_policy,
            &serde_json::to_vec_pretty(&network.dns_policy())?,
        )?;
        let launcher = handle.directory.join("launch-clean-scan.py");
        write_new(&launcher, include_bytes!("../scripts/launch-clean-scan.py"))?;
        let python = [
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
            "/usr/bin/python3",
        ]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .context("Python 3 is required for the independent check supervisor")?;
        let input_hash = sha256(&handle.input())?;
        let binary_hash = sha256(&handle.directory.join("bsync"))?;
        let dns_hash = sha256(&dns_policy)?;
        for verify in [true, false] {
            let mut command = Command::new(python);
            command
                .arg(&launcher)
                .arg("--batch")
                .arg(&handle.directory)
                .arg("--input-sha256")
                .arg(&input_hash)
                .arg("--binary-sha256")
                .arg(&binary_hash)
                .arg("--timeout")
                .arg(network.timeout_secs.to_string())
                .arg("--dns-policy")
                .arg(&dns_policy)
                .arg("--dns-policy-sha256")
                .arg(&dns_hash);
            if let Some(proxy) = network.proxy_url() {
                command.arg("--proxy").arg(proxy);
            } else {
                command.arg("--direct");
            }
            if verify {
                command.arg("--verify-only");
            }
            let output = command.output().context("Launch check supervisor")?;
            ensure!(
                output.status.success(),
                "Check launch refused: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(handle)
    }
}

#[cfg(target_os = "macos")]
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn sha256(path: &Path) -> Result<String> {
    let output = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()?;
    ensure!(output.status.success(), "Could not hash the check copy");
    let hash = String::from_utf8(output.stdout)?
        .split_whitespace()
        .next()
        .context("SHA-256 output missing")?
        .to_owned();
    ensure!(
        hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA-256 output"
    );
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_network_is_rejected_before_batch_preparation() {
        let network = NetworkSettings {
            timeout_secs: 0,
            ..NetworkSettings::default()
        };
        let error = begin(Path::new("nonexistent-input"), "unused", None, network)
            .err()
            .unwrap();
        assert!(error.to_string().contains("Request timeout"));
    }
    #[test]
    fn missing_receipt_is_not_completion_and_abort_is_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let handle = ScanHandle {
            directory: directory.path().into(),
        };
        assert_eq!(handle.status().unwrap().state, "starting");
        assert!(handle.verify_complete().is_err());
        fs::create_dir(directory.path().join("health")).unwrap();
        fs::write(
            directory.path().join("health/check-status.json"),
            br#"{"status":"aborted","phase":"site","completed_targets":9,"total_targets":10}"#,
        )
        .unwrap();
        let state = handle.status().unwrap();
        assert_eq!(state.state, "aborted");
        assert_eq!(state.completed, Some(9));
        assert!(handle.verify_complete().is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn successful_exit_requires_every_planned_site_and_content_target() {
        let directory = tempfile::tempdir().unwrap();
        let handle = ScanHandle {
            directory: directory.path().into(),
        };
        fs::create_dir(directory.path().join("health")).unwrap();
        record_input(&handle);
        fs::write(
            directory.path().join("process-exit.json"),
            br#"{"process_exit_code":0}"#,
        )
        .unwrap();
        fs::write(
            directory.path().join("health/check-status.json"),
            br#"{"status":"completed","phase":"content","completed_targets":1,"total_targets":1}"#,
        )
        .unwrap();
        fs::write(directory.path().join("health/probe-plan.json"), br#"{"scope":"content","partial":false,"targets":[{"probe_url":"https://example.com/page"}],"site_preflight":{"targets":[{"probe_url":"https://example.com/"}]}}"#).unwrap();
        let conn = Connection::open(handle.db()).unwrap();
        conn.execute_batch("CREATE TABLE health_runs(run_id TEXT,completed INTEGER); CREATE TABLE health_checks(run_id TEXT,bookmark_id TEXT,original_url TEXT,finished_at TEXT); INSERT INTO health_runs VALUES('run',1); INSERT INTO health_checks VALUES('run','site:example','https://example.com/','now');").unwrap();
        assert!(handle.verify_complete().is_err());
        conn.execute(
            "INSERT INTO health_checks VALUES('run','bookmark','https://example.com/page','now')",
            [],
        )
        .unwrap();
        assert!(handle.verify_complete().is_ok());
        fs::write(handle.input(), "changed input").unwrap();
        assert!(handle.verify_complete().is_err());
        fs::write(handle.input(), "original input").unwrap();
        conn.execute("UPDATE health_runs SET completed=0", [])
            .unwrap();
        assert!(handle.verify_complete().is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn completed_empty_content_plan_does_not_require_a_nonexistent_phase() {
        let directory = tempfile::tempdir().unwrap();
        let handle = ScanHandle {
            directory: directory.path().into(),
        };
        fs::create_dir(directory.path().join("health")).unwrap();
        record_input(&handle);
        fs::write(
            directory.path().join("process-exit.json"),
            br#"{"process_exit_code":0}"#,
        )
        .unwrap();
        fs::write(
            directory.path().join("health/check-status.json"),
            br#"{"status":"completed","phase":"site","completed_targets":0,"total_targets":0}"#,
        )
        .unwrap();
        fs::write(
            directory.path().join("health/probe-plan.json"),
            br#"{"scope":"content","partial":false,"targets":[],"site_preflight":{"targets":[]}}"#,
        )
        .unwrap();
        Connection::open(handle.db()).unwrap().execute_batch("CREATE TABLE health_runs(run_id TEXT,completed INTEGER); CREATE TABLE health_checks(run_id TEXT,bookmark_id TEXT,original_url TEXT,finished_at TEXT);").unwrap();
        assert!(handle.verify_complete().is_ok());
        fs::write(
            directory.path().join("process-exit.json"),
            br#"{"process_exit_code":1}"#,
        )
        .unwrap();
        assert_eq!(handle.status().unwrap().state, "failed");
        assert!(handle.verify_complete().is_err());
    }

    #[cfg(target_os = "macos")]
    fn record_input(handle: &ScanHandle) {
        fs::write(handle.input(), "original input").unwrap();
        let manifest = serde_json::json!({"input_sha256":sha256(&handle.input()).unwrap()});
        fs::write(
            handle.directory.join("run-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }
}
