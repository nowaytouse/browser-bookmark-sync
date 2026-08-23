//! ffuf CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::{reject_unsafe_target, run_command_output, ProbeBackend};
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct FfufCliBackend;

impl FfufCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("FFUF_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/opt/homebrew/bin/ffuf", "/usr/local/bin/ffuf", "ffuf"] {
            if Command::new(candidate).arg("-V").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for FfufCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for FfufCliBackend {
    fn backend_id(&self) -> &'static str {
        "ffuf"
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

        if reject_unsafe_target(target, context, &mut obs) {
            return Ok(obs);
        }

        let bin_path = if let Some(b) = Self::find_binary() {
            b
        } else {
            obs.backend_version = "not_installed".to_string();
            obs.actual_request = "unknown".to_string();
            obs.error_kind = Some("ToolNotInstalled: ffuf".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "2.1.0".to_string();

        let mut command = Command::new(&bin_path);
        command
            .arg("-u")
            .arg(&target.original_url)
            .arg("-s")
            .arg("-json")
            .arg("-timeout")
            .arg(format!("{}", context.timeout_secs));
        let output = run_command_output(command).await;

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                let trim_stdout = stdout_str.trim();

                if let Ok(v) = serde_json::from_str::<Value>(trim_stdout) {
                    if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
                        if let Some(first) = results.first() {
                            if let Some(status) = first.get("status").and_then(Value::as_u64) {
                                obs.status_code = Some(status as u16);
                            }
                            if let Some(redir) =
                                first.get("redirectlocation").and_then(|r| r.as_str())
                            {
                                if !redir.is_empty() {
                                    obs.final_url = Some(redir.to_string());
                                }
                            }
                        }
                    } else if let Some(status) = v.get("status").and_then(Value::as_u64) {
                        obs.status_code = Some(status as u16);
                    }
                } else if out.status.success() {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some(format!("FfufExitCode: {:?}", out.status.code()));
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("FfufExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}
