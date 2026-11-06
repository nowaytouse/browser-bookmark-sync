//! Ping CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::{reject_unsafe_network_target, run_command_output, ProbeBackend};
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct PingCliBackend;

impl PingCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        if let Ok(path) = std::env::var("PING_BIN") {
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        for candidate in &["/sbin/ping", "/usr/bin/ping", "ping"] {
            if Command::new(candidate).arg("-?").output().is_ok()
                || Command::new(candidate).arg("-h").output().is_ok()
                || Command::new(candidate).output().is_ok()
            {
                return Some((*candidate).to_string());
            }
        }
        None
    }
}

impl Default for PingCliBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for PingCliBackend {
    fn backend_id(&self) -> &'static str {
        "ping"
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

        let bin_path = if let Some(b) = Self::find_binary() {
            b
        } else {
            obs.backend_version = "not_installed".to_string();
            obs.actual_request = "unknown".to_string();
            obs.error_kind = Some("ToolNotInstalled: ping".to_string());
            obs.finished_at = Utc::now();
            return Ok(obs);
        };

        obs.backend_version = "1.0.0".to_string();

        let mut cmd = Command::new(&bin_path);
        cmd.arg("-c").arg("1");

        // macOS uses -W in ms, Linux uses -w in seconds
        if cfg!(target_os = "macos") {
            cmd.arg("-W").arg("2000");
        } else {
            cmd.arg("-w").arg("2");
        }

        cmd.arg(&target.host);
        let output = run_command_output(cmd).await;

        obs.finished_at = Utc::now();

        match output {
            Ok(out) => {
                obs.actual_request = "yes".to_string();
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                if out.status.success()
                    || stdout_str.contains("0.0% packet loss")
                    || stdout_str.contains("0% packet loss")
                {
                    obs.status_code = Some(200);
                } else {
                    obs.error_kind = Some("IcmpUnreachable".to_string());
                }
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("PingExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}
