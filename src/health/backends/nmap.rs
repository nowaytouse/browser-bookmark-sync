//! Nmap CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct NmapCliBackend;

impl NmapCliBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("NMAP_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/usr/local/bin/nmap", "/opt/homebrew/bin/nmap", "nmap"] {
            if Command::new(candidate).arg("--version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for NmapCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for NmapCliBackend {
    fn backend_id(&self) -> &'static str {
        "nmap"
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

        let bin_path = match Self::find_binary() {
            Some(b) => b,
            None => {
                obs.backend_version = "not_installed".to_string();
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some("ToolNotInstalled: nmap".to_string());
                obs.finished_at = Utc::now();
                return Ok(obs);
            }
        };

        obs.backend_version = "7.94".to_string();

        let port = target
            .port
            .unwrap_or(if target.scheme == "https" { 443 } else { 80 });

        let output = Command::new(&bin_path)
            .arg("-p")
            .arg(format!("{port}"))
            .arg("-Pn")
            .arg("--host-timeout")
            .arg(format!("{}s", context.timeout_secs))
            .arg(&target.host)
            .output();

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                if stdout_str.contains("open") {
                    obs.status_code = Some(200);
                } else if stdout_str.contains("closed") {
                    obs.error_kind = Some("PortClosed".to_string());
                } else if stdout_str.contains("filtered") {
                    obs.error_kind = Some("PortFiltered".to_string());
                } else if out.status.success() {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some(format!("NmapExitCode: {:?}", out.status.code()));
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("NmapExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}
