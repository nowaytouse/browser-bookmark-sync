//! curl CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct CurlCliBackend;

impl CurlCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("CURL_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/usr/bin/curl", "/opt/homebrew/bin/curl", "curl"] {
            if Command::new(candidate).arg("--version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for CurlCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for CurlCliBackend {
    fn backend_id(&self) -> &'static str {
        "curl"
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
            obs.error_kind = Some("ToolNotInstalled: curl".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "8.7.1".to_string();

        let output = Command::new(&bin_path)
            .arg("-s")
            .arg("-L")
            .arg("--max-redirs")
            .arg("10")
            .arg("--connect-timeout")
            .arg("10")
            .arg("-m")
            .arg(format!("{}", context.timeout_secs))
            .arg("-o")
            .arg("/dev/null")
            .arg("-w")
            .arg("%{json}")
            .arg(&target.original_url)
            .output();

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                if let Ok(v) = serde_json::from_str::<Value>(&stdout_str) {
                    if let Some(code) = v.get("http_code").and_then(Value::as_u64) {
                        if code > 0 {
                            obs.status_code = Some(code as u16);
                        }
                    }
                    if let Some(time) = v.get("time_total").and_then(Value::as_f64) {
                        obs.response_time_ms = (time * 1000.0) as u64;
                    }
                    if let Some(url_effective) = v.get("url_effective").and_then(|u| u.as_str()) {
                        if url_effective != target.original_url {
                            obs.final_url = Some(url_effective.to_string());
                        }
                    }
                    if let Some(ip) = v.get("remote_ip").and_then(|i| i.as_str()) {
                        if !ip.is_empty() {
                            obs.ip_addresses.push(ip.to_string());
                        }
                    }

                    if !out.status.success() && obs.status_code.is_none() {
                        obs.error_kind = Some(format!("CurlExitCode: {:?}", out.status.code()));
                    }
                } else if out.status.success() {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some(format!("CurlExitCode: {:?}", out.status.code()));
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("CurlExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}
