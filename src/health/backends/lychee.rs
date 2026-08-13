//! Lychee CLI Probe Backend Adapter (Tier 2)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::process::Command;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Copy)]
pub struct LycheeCliBackend;

impl LycheeCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("LYCHEE_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &[
            "/opt/homebrew/bin/lychee",
            "/usr/local/bin/lychee",
            "lychee",
        ] {
            if Command::new(candidate).arg("--version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for LycheeCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for LycheeCliBackend {
    fn backend_id(&self) -> &'static str {
        "lychee"
    }

    async fn probe(
        &self,
        target: &ProbeTarget,
        context: &ProbeContext,
    ) -> Result<BackendObservation, String> {
        let started_at = Utc::now();
        let mut obs = BackendObservation::default();
        obs.backend = self.backend_id().to_string();
        obs.run_id = context.run_id.clone();
        obs.check_id = context.check_id.clone();
        obs.started_at = started_at;
        obs.initial_url = target.original_url.clone();

        let bin_path = if let Some(b) = Self::find_binary() {
            b
        } else {
            obs.backend_version = "not_installed".to_string();
            obs.actual_request = "unknown".to_string();
            obs.error_kind = Some("ToolNotInstalled: lychee".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "0.24.2".to_string();

        // Write target URL to a temporary input file
        let mut temp_file = NamedTempFile::new().map_err(|e| format!("Temp file error: {e}"))?;
        use std::io::Write;
        writeln!(temp_file, "{}", target.original_url)
            .map_err(|e| format!("Temp write error: {e}"))?;

        let temp_path = temp_file.path().to_string_lossy().to_string();

        let output = Command::new(&bin_path)
            .arg("--format")
            .arg("json")
            .arg("--max-redirects")
            .arg("10")
            .arg("--timeout")
            .arg(format!("{}", context.timeout_secs))
            .arg(&temp_path)
            .output();

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                let code = out.status.code().unwrap_or(-1);
                let stdout_str = String::from_utf8_lossy(&out.stdout);

                // Exit code 0 (all ok) or Exit code 2 (found broken links) are valid command completions!
                if code == 0 || code == 2 {
                    obs.actual_request = "yes".to_string();
                    if let Ok(v) = serde_json::from_str::<Value>(&stdout_str) {
                        if code == 0 {
                            obs.status_code = Some(200);
                        } else {
                            obs.status_code = Some(404);
                            obs.error_kind = Some("LycheeDetectedFailure".to_string());
                            if let Some(fail_map) = v.get("fail_map").or_else(|| v.get("results")) {
                                obs.body_preview = Some(fail_map.to_string());
                            }
                        }
                    } else if code == 0 {
                        obs.status_code = Some(200);
                    } else {
                        // Exit code 2 but malformed JSON -> process failure
                        obs.actual_request = "unknown".to_string();
                        obs.error_kind = Some("LycheeMalformedJson".to_string());
                    }
                } else {
                    // Exit code 1 or other error
                    obs.actual_request = "unknown".to_string();
                    obs.error_kind = Some(format!("LycheeExitCode: {code}"));
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("LycheeExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}
