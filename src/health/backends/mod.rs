//! Probe Backend Trait and Implementations

pub mod curl;
pub mod dnsx;
pub mod ffuf;
pub mod httpx;
pub mod lychee;
pub mod native_dns;
pub mod native_reqwest;
pub mod nmap;
pub mod openssl;
pub mod ping;
pub mod wget;

use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use std::process::{Command, Output};

/// Run a potentially long-lived external command without blocking a Tokio worker.
pub(crate) async fn run_command_output(mut command: Command) -> std::io::Result<Output> {
    tokio::task::spawn_blocking(move || command.output())
        .await
        .map_err(|error| std::io::Error::other(format!("command task failed: {error}")))?
}

/// Reject unsafe targets before invoking an external network utility.
pub(crate) fn reject_unsafe_target(
    target: &ProbeTarget,
    context: &ProbeContext,
    observation: &mut BackendObservation,
) -> bool {
    if let crate::health::security::SecurityCheckResult::Denied(reason) =
        crate::health::security::validate_url_security(
            &target.original_url,
            context.allow_private_ips,
        )
    {
        observation.actual_request = "no".to_string();
        observation.error_kind = Some(format!("SecurityBlocked: {reason}"));
        observation.finished_at = Utc::now();
        true
    } else {
        false
    }
}

#[async_trait]
pub trait ProbeBackend: Send + Sync {
    fn backend_id(&self) -> &'static str;

    async fn probe(
        &self,
        target: &ProbeTarget,
        context: &ProbeContext,
    ) -> Result<BackendObservation, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> ProbeContext {
        ProbeContext {
            run_id: "run".to_string(),
            check_id: "check".to_string(),
            proxy_url: None,
            timeout_secs: 10,
            slow_mode: false,
            allow_private_ips: false,
            history_db_path: None,
            secondary_backends: Vec::new(),
            artifact_dir: None,
        }
    }

    #[test]
    fn external_targets_allow_http_s_only() {
        let context = context();
        for url in ["http://example.com", "https://example.com"] {
            let target = ProbeTarget::from_url("bookmark", url).unwrap();
            let mut observation = BackendObservation::default();
            assert!(!reject_unsafe_target(&target, &context, &mut observation));
        }

        for url in ["ftp://example.com/file", "file:///tmp/bookmarks"] {
            let target = ProbeTarget::from_url("bookmark", url).unwrap();
            let mut observation = BackendObservation::default();
            assert!(reject_unsafe_target(&target, &context, &mut observation));
            assert_eq!(observation.actual_request, "no");
        }
    }

    #[test]
    fn external_targets_block_private_hosts_by_default() {
        let target = ProbeTarget::from_url("bookmark", "http://127.0.0.1").unwrap();
        let mut observation = BackendObservation::default();
        assert!(reject_unsafe_target(&target, &context(), &mut observation));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn external_command_wait_does_not_block_tokio_worker() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 1"]);

        let started = std::time::Instant::now();
        let timer = async {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            started.elapsed()
        };
        let (output, timer_elapsed) = tokio::join!(run_command_output(command), timer);

        assert!(output.unwrap().status.success());
        assert!(
            timer_elapsed < std::time::Duration::from_millis(750),
            "Tokio timer was blocked for {timer_elapsed:?}"
        );
    }
}
