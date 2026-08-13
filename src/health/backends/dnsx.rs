//! `ProjectDiscovery` dnsx CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct DnsxCliBackend {
    resolver_id: &'static str,
}

impl DnsxCliBackend {
    pub const fn new() -> Self {
        Self::cloudflare()
    }

    pub const fn cloudflare() -> Self {
        Self {
            resolver_id: "1.1.1.1",
        }
    }

    pub const fn alidns() -> Self {
        Self {
            resolver_id: "223.5.5.5",
        }
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("DNSX_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/opt/homebrew/bin/dnsx", "/usr/local/bin/dnsx", "dnsx"] {
            if Command::new(candidate).arg("-version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for DnsxCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for DnsxCliBackend {
    fn backend_id(&self) -> &'static str {
        "dnsx"
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
        obs.resolver_id = Some(self.resolver_id.to_string());

        let bin_path = match Self::find_binary() {
            Some(path) => path,
            None => {
                obs.backend_version = "not_installed".to_string();
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some("ToolNotInstalled: dnsx".to_string());
                obs.finished_at = Utc::now();
                return Ok(obs);
            }
        };

        obs.backend_version = "1.1.6".to_string();

        let output = Command::new(&bin_path)
            .arg("-d")
            .arg(&target.host)
            .arg("-a")
            .arg("-r")
            .arg(self.resolver_id)
            .arg("-json")
            .output();

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                let trim_stdout = stdout_str.trim();

                if !trim_stdout.is_empty() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trim_stdout) {
                        obs.actual_request = "yes".to_string();
                        if let Some(rcode) = v
                            .get("status_code")
                            .or_else(|| v.get("rcode"))
                            .and_then(|r| r.as_str())
                        {
                            obs.dns_rcode = Some(rcode.to_string());
                            if rcode.eq_ignore_ascii_case("NXDOMAIN") {
                                obs.error_kind = Some("DnsNxdomain".to_string());
                            } else if rcode.eq_ignore_ascii_case("SERVFAIL") {
                                obs.error_kind = Some("DnsServfail".to_string());
                            }
                        }
                    } else {
                        obs.actual_request = "unknown".to_string();
                        obs.error_kind = Some("DnsxMalformedJson".to_string());
                    }
                } else if out.status.success() {
                    obs.actual_request = "yes".to_string();
                    obs.dns_rcode = Some("NOERROR".to_string());
                } else {
                    // Empty stdout and process failure is NOT NXDOMAIN!
                    obs.actual_request = "unknown".to_string();
                    obs.error_kind = Some("DnsxExecutionFailed".to_string());
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("DnsxError: {e}"));
            }
        }

        Ok(obs)
    }
}
