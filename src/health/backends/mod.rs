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
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const MAX_COMMAND_OUTPUT_BYTES: u64 = 1024 * 1024;
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Run a potentially long-lived external command without blocking a Tokio worker.
pub(crate) async fn run_command_output(command: Command) -> std::io::Result<Output> {
    run_command_output_with_timeout(command, DEFAULT_COMMAND_TIMEOUT).await
}

/// Drain both pipes concurrently, cap captured output, and kill a stuck utility.
pub(crate) async fn run_command_output_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> std::io::Result<Output> {
    tokio::task::spawn_blocking(move || {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("child stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("child stderr was not piped"))?;

        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(MAX_COMMAND_OUTPUT_BYTES + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_COMMAND_OUTPUT_BYTES + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "external command exceeded {} seconds",
                        timeout.as_secs_f64()
                    ),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        let stdout = stdout_reader
            .join()
            .map_err(|_| std::io::Error::other("stdout reader panicked"))??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| std::io::Error::other("stderr reader panicked"))??;
        if stdout.len() as u64 > MAX_COMMAND_OUTPUT_BYTES
            || stderr.len() as u64 > MAX_COMMAND_OUTPUT_BYTES
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "external command output exceeded 1 MiB",
            ));
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    })
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

fn blocked_resolved_ip(ips: &[std::net::IpAddr], allow_private: bool) -> Option<String> {
    ips.iter()
        .find_map(|ip| crate::health::security::is_ip_allowed(*ip, allow_private).err())
}

/// Resolve a target before handing it to a utility that would otherwise choose an
/// unchecked address itself. DNS errors remain non-destructive, review-only evidence.
pub(crate) async fn reject_unsafe_network_target(
    target: &ProbeTarget,
    context: &ProbeContext,
    observation: &mut BackendObservation,
) -> bool {
    if reject_unsafe_target(target, context, observation) {
        return true;
    }
    let port = target
        .port
        .unwrap_or(if target.scheme == "https" { 443 } else { 80 });
    let lookup = tokio::time::timeout(
        Duration::from_secs(context.timeout_secs.clamp(1, 15)),
        tokio::net::lookup_host((target.host.as_str(), port)),
    )
    .await;
    let mut ips: Vec<_> = match lookup {
        Ok(Ok(addresses)) => addresses.map(|address| address.ip()).collect(),
        Ok(Err(error)) => {
            observation.actual_request = "no".to_string();
            observation.error_kind = Some(format!("DnsPreflightError: {error}"));
            observation.finished_at = Utc::now();
            return true;
        }
        Err(_) => {
            observation.actual_request = "no".to_string();
            observation.error_kind = Some("DnsPreflightTimeout".to_string());
            observation.finished_at = Utc::now();
            return true;
        }
    };
    ips.sort_unstable();
    ips.dedup();
    if ips.is_empty() {
        observation.actual_request = "no".to_string();
        observation.error_kind = Some("DnsPreflightNoAddresses".to_string());
        observation.finished_at = Utc::now();
        return true;
    }
    observation.ip_addresses = ips.iter().map(ToString::to_string).collect();
    if let Some(reason) = blocked_resolved_ip(&ips, context.allow_private_ips) {
        observation.actual_request = "no".to_string();
        observation.error_kind = Some(format!("SecurityBlocked: {reason}"));
        observation.finished_at = Utc::now();
        return true;
    }
    false
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
            probe_scope: crate::health::targets::ProbeScope::Content,
            dns_policy_hash: "test-dns-policy".to_string(),
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

        for url in ["ftp://example.com/file"] {
            let target = ProbeTarget::from_url("bookmark", url).unwrap();
            let mut observation = BackendObservation::default();
            assert!(reject_unsafe_target(&target, &context, &mut observation));
            assert_eq!(observation.actual_request, "no");
        }
        assert!(ProbeTarget::from_url("bookmark", "file:///tmp/bookmarks").is_err());
    }

    #[test]
    fn external_targets_block_private_hosts_by_default() {
        let target = ProbeTarget::from_url("bookmark", "http://127.0.0.1").unwrap();
        let mut observation = BackendObservation::default();
        assert!(reject_unsafe_target(&target, &context(), &mut observation));
    }

    #[test]
    fn external_tools_reject_any_private_address_in_a_dns_answer() {
        let ips = [
            "93.184.216.34".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
        ];
        assert!(blocked_resolved_ip(&ips, false).is_some());
        assert!(blocked_resolved_ip(&ips, true).is_none());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn external_command_wait_does_not_block_tokio_worker() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 1"]);

        let started = Instant::now();
        let timer = async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            started.elapsed()
        };
        let (output, timer_elapsed) = tokio::join!(run_command_output(command), timer);

        assert!(output.unwrap().status.success());
        assert!(
            timer_elapsed < Duration::from_millis(750),
            "Tokio timer was blocked for {timer_elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn external_command_is_killed_at_the_hard_timeout() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        let error = run_command_output_with_timeout(command, Duration::from_millis(25))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }
}
