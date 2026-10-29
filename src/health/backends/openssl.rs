//! OpenSSL CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::{
    reject_unsafe_network_target, run_command_output_with_timeout, ProbeBackend,
};
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct OpenSslCliBackend;

impl OpenSslCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("OPENSSL_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/opt/homebrew/bin/openssl", "/usr/bin/openssl", "openssl"] {
            if Command::new(candidate).arg("version").output().is_ok() {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for OpenSslCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for OpenSslCliBackend {
    fn backend_id(&self) -> &'static str {
        "openssl"
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

        if reject_unsafe_network_target(target, context, &mut obs).await {
            return Ok(obs);
        }

        if target.scheme != "https" {
            obs.actual_request = "no".to_string();
            obs.error_kind = Some("NotHttpsScheme".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        }

        let bin_path = if let Some(b) = Self::find_binary() {
            b
        } else {
            obs.backend_version = "not_installed".to_string();
            obs.actual_request = "unknown".to_string();
            obs.error_kind = Some("ToolNotInstalled: openssl".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "3.6.3".to_string();

        let host_port = format!("{}:{}", target.host, target.port.unwrap_or(443));

        let mut command = Command::new(&bin_path);
        command
            .arg("s_client")
            .arg("-connect")
            .arg(&host_port)
            .arg("-servername")
            .arg(&target.host)
            .arg("-brief");
        let output_res = run_command_output_with_timeout(
            command,
            std::time::Duration::from_secs(context.timeout_secs.clamp(1, 60)),
        )
        .await;

        obs.finished_at = Utc::now();

        match output_res {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stderr_str = String::from_utf8_lossy(&out.stderr);
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                let combined = format!("{stdout_str}\n{stderr_str}");

                let summary = combined.lines().take(5).collect::<Vec<_>>().join(" ");

                obs.tls_summary = Some(summary);

                if out.status.success() || combined.contains("Verification: OK") {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some("TlsHandshakeFailed".to_string());
                }
            }
            Err(error) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(if error.kind() == std::io::ErrorKind::TimedOut {
                    "OpenSslTimeout".to_string()
                } else {
                    format!("OpenSslExecutionError: {error}")
                });
            }
        }

        Ok(obs)
    }
}
