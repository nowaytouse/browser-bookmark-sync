//! `ProjectDiscovery` httpx CLI Probe Backend Adapter (Tier 3 & Tier 4)

use crate::health::backends::{reject_unsafe_target, run_command_output, ProbeBackend};
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::process::Command;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Copy)]
pub struct HttpxCliBackend;

impl HttpxCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("HTTPX_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/opt/homebrew/bin/httpx", "/usr/local/bin/httpx", "httpx"] {
            if Command::new(candidate).arg("-version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for HttpxCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for HttpxCliBackend {
    fn backend_id(&self) -> &'static str {
        "httpx"
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
            obs.error_kind = Some("ToolNotInstalled: httpx".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "1.6.0".to_string();

        let mut temp_file = NamedTempFile::new().map_err(|e| format!("Temp file error: {e}"))?;
        use std::io::Write;
        writeln!(temp_file, "{}", target.original_url)
            .map_err(|e| format!("Temp write error: {e}"))?;

        let temp_path = temp_file.path().to_string_lossy().to_string();

        let mut command = Command::new(&bin_path);
        command
            .arg("-l")
            .arg(&temp_path)
            .arg("-json")
            .arg("-status-code")
            .arg("-title")
            .arg("-content-length")
            .arg("-content-type")
            .arg("-timeout")
            .arg(format!("{}", context.timeout_secs));
        let output = run_command_output(command).await;

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                if let Ok(v) = serde_json::from_str::<Value>(&stdout_str) {
                    if let Some(code) = v.get("status_code").and_then(Value::as_u64) {
                        obs.status_code = Some(code as u16);
                    }
                    if let Some(t) = v.get("title").and_then(|t| t.as_str()) {
                        obs.title = Some(t.to_string());
                    }
                    if let Some(loc) = v.get("location").and_then(|l| l.as_str()) {
                        obs.final_url = Some(loc.to_string());
                    }
                    if let Some(cl) = v.get("content_length").and_then(Value::as_u64) {
                        obs.content_length = Some(cl);
                    }
                    if let Some(ct) = v.get("content_type").and_then(|t| t.as_str()) {
                        obs.content_type = Some(ct.to_string());
                    }
                } else if out.status.success() {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some("HttpxExecutionFailed".to_string());
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("HttpxError: {e}"));
            }
        }

        if obs
            .status_code
            .is_some_and(|code| (200..300).contains(&code))
            && obs.error_kind.is_none()
        {
            if crate::health::content::is_parked_domain(obs.title.as_deref(), None) {
                obs.error_kind = Some("ContentMissing".to_string());
            } else if crate::health::content::is_soft_404(obs.title.as_deref(), None) {
                obs.error_kind = Some("Soft404Candidate".to_string());
            }
        }

        Ok(obs)
    }
}
