//! Wget CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct WgetCliBackend;

impl WgetCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("WGET_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &[
            "/usr/bin/wget",
            "/opt/homebrew/bin/wget",
            "/usr/local/bin/wget",
            "wget",
        ] {
            if Command::new(candidate).arg("--version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }

    pub fn parse_http_status(stderr_str: &str) -> Option<u16> {
        for line in stderr_str.lines() {
            for token in line.split_whitespace() {
                let clean = token.trim_matches(|c: char| !c.is_ascii_digit());
                if clean.len() == 3 {
                    if let Ok(code) = clean.parse::<u16>() {
                        if (100..=599).contains(&code) {
                            return Some(code);
                        }
                    }
                }
            }
        }
        None
    }
}

impl Default for WgetCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for WgetCliBackend {
    fn backend_id(&self) -> &'static str {
        "wget"
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
            obs.error_kind = Some("ToolNotInstalled: wget".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "1.21.4".to_string();

        let output = Command::new(&bin_path)
            .arg("--spider")
            .arg("-S")
            .arg("-t")
            .arg("1")
            .arg("-T")
            .arg(format!("{}", context.timeout_secs))
            .arg("--no-check-certificate")
            .arg(&target.original_url)
            .output();

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stderr_str = String::from_utf8_lossy(&out.stderr);
                if let Some(code) = Self::parse_http_status(&stderr_str) {
                    obs.status_code = Some(code);
                } else if out.status.success() {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some(format!("WgetExitCode: {:?}", out.status.code()));
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("WgetExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}
