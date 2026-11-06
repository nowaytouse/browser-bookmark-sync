//! curl CLI Probe Backend Adapter (Tier 5)

use crate::health::backends::{reject_unsafe_target, ProbeBackend};
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::io::Read;
use std::net::IpAddr;
use std::process::Command;
use std::sync::OnceLock;

static CURL_BINARY: OnceLock<Option<String>> = OnceLock::new();
const MAX_BODY_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct CurlCliBackend;

impl CurlCliBackend {
    pub const fn new() -> Self {
        Self
    }

    pub fn find_binary() -> Option<String> {
        CURL_BINARY.get_or_init(Self::discover_binary).clone()
    }

    fn discover_binary() -> Option<String> {
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

    fn observe_html_content(observation: &mut BackendObservation, target_url: &str, body: &[u8]) {
        let is_html = observation
            .content_type
            .as_ref()
            .is_some_and(|content_type| {
                let lower = content_type.to_lowercase();
                lower.contains("html") || lower.contains("xhtml")
            });
        if !is_html || body.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(body);
        observation.title = crate::health::content::extract_html_title(&text);
        observation.canonical_url = crate::health::content::extract_canonical_url(&text);
        observation.body_hash = Some(blake3::hash(body).to_hex().to_string());
        observation.body_preview = crate::health::content::extract_content_text(&text)
            .or_else(|| Some(text.chars().take(500).collect()));

        if observation.error_kind.is_none() {
            if crate::health::content::is_parked_domain(
                observation.title.as_deref(),
                observation.body_preview.as_deref(),
            ) {
                observation.error_kind = Some("ParkedDomainCandidate".to_string());
            } else if let Some(issue) =
                crate::health::content::structured_content_error(target_url, &text)
            {
                observation.error_kind = Some(issue.to_string());
            } else if crate::health::content::is_soft_404(
                observation.title.as_deref(),
                observation.body_preview.as_deref(),
            ) {
                observation.error_kind = Some("Soft404Candidate".to_string());
            }
        }
    }

    fn configure_route(
        command: &mut Command,
        proxy_url: Option<&str>,
        resolve_entry: Option<&str>,
    ) {
        if let Some(proxy_url) = proxy_url {
            command
                .env_remove("NO_PROXY")
                .env_remove("no_proxy")
                .arg("--proxy")
                .arg(proxy_url);
        } else {
            command.arg("--noproxy").arg("*");
            if let Some(resolve_entry) = resolve_entry {
                command.arg("--resolve").arg(resolve_entry);
            }
        }
    }

    fn build_command(
        binary: &str,
        target_url: &str,
        body_path: &std::path::Path,
        timeout_secs: u64,
        proxy_url: Option<&str>,
        resolve_entry: Option<&str>,
    ) -> Command {
        let mut command = Command::new(binary);
        command
            // This must be first: local curlrc must not add requests, credentials or uploads.
            .arg("-q")
            .arg("--globoff")
            .arg("-s")
            .arg("--proto")
            .arg("=http,https")
            .arg("--max-redirs")
            .arg("0")
            .arg("--max-filesize")
            .arg(MAX_BODY_BYTES.to_string())
            .arg("--connect-timeout")
            .arg("10")
            .arg("-m")
            .arg(format!("{timeout_secs}"))
            .arg("-o")
            .arg(body_path)
            .arg("-w")
            .arg("%{json}\n%header{retry-after}");
        Self::configure_route(&mut command, proxy_url, resolve_entry);
        command.arg("--url").arg(target_url);
        command
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
        obs.request_method = "GET".to_string();
        obs.route = if context.proxy_url.is_some() {
            "proxy".to_string()
        } else {
            "direct".to_string()
        };

        if reject_unsafe_target(target, context, &mut obs) {
            return Ok(obs);
        }

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

        let target_url = target.original_url.clone();
        let timeout_secs = context.timeout_secs;
        let proxy_url = context.proxy_url.clone();
        let resolve_entry = if proxy_url.is_some() {
            None
        } else {
            let port = target
                .port
                .unwrap_or(if target.scheme == "https" { 443 } else { 80 });
            let mut resolved_ips: Vec<IpAddr> =
                match tokio::net::lookup_host((target.host.as_str(), port)).await {
                    Ok(addresses) => addresses.map(|address| address.ip()).collect(),
                    Err(error) => {
                        obs.actual_request = "no".to_string();
                        obs.error_kind = Some(format!("DnsError: {error}"));
                        obs.finished_at = Utc::now();
                        return Ok(obs);
                    }
                };
            resolved_ips.sort_unstable();
            resolved_ips.dedup();
            if resolved_ips.is_empty() {
                obs.actual_request = "no".to_string();
                obs.error_kind = Some("DnsError: no addresses".to_string());
                obs.finished_at = Utc::now();
                return Ok(obs);
            }
            if let Some(reason) = resolved_ips.iter().find_map(|ip| {
                crate::health::security::is_ip_allowed(*ip, context.allow_private_ips).err()
            }) {
                obs.actual_request = "no".to_string();
                obs.error_kind = Some(format!("SecurityBlocked: {reason}"));
                obs.finished_at = Utc::now();
                return Ok(obs);
            }
            obs.ip_addresses = resolved_ips.iter().map(ToString::to_string).collect();
            let pinned_ip = match resolved_ips[0] {
                IpAddr::V4(ip) => ip.to_string(),
                IpAddr::V6(ip) => format!("[{ip}]"),
            };
            Some(format!("{}:{port}:{pinned_ip}", target.host))
        };
        let content_target_url = target_url.clone();
        let output = tokio::task::spawn_blocking(move || {
            let body_file = tempfile::NamedTempFile::new()?;
            let mut command = Self::build_command(
                &bin_path,
                &target_url,
                body_file.path(),
                timeout_secs,
                proxy_url.as_deref(),
                resolve_entry.as_deref(),
            );
            let command_output = command.output()?;
            let mut body = Vec::new();
            std::fs::File::open(body_file.path())?
                .take(MAX_BODY_BYTES)
                .read_to_end(&mut body)?;
            Ok::<_, std::io::Error>((command_output, body))
        })
        .await;

        obs.finished_at = Utc::now();

        match output {
            Ok(Ok((out, body))) => {
                obs.actual_request = "yes".to_string();
                let body_too_large = out.status.code() == Some(63);
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                let (metadata, retry_after) =
                    stdout_str.split_once('\n').unwrap_or((&stdout_str, ""));
                if let Ok(v) = serde_json::from_str::<Value>(metadata) {
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
                        // With an explicit proxy this is the proxy peer, not the target IP.
                        // Target URL validation still runs before starting curl.
                        if !ip.is_empty() && context.proxy_url.is_none() {
                            if let Ok(parsed_ip) = ip.parse::<IpAddr>() {
                                if let Err(reason) = crate::health::security::is_ip_allowed(
                                    parsed_ip,
                                    context.allow_private_ips,
                                ) {
                                    obs.actual_request = "no".to_string();
                                    obs.error_kind = Some(format!("SecurityBlocked: {reason}"));
                                }
                            }
                            obs.ip_addresses = vec![ip.to_string()];
                        }
                    }
                    obs.content_type = v
                        .get("content_type")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);

                    if body_too_large {
                        obs.actual_request = "unknown".to_string();
                        obs.error_kind = Some("BodyTooLarge".to_string());
                    } else if !out.status.success() && obs.error_kind.is_none() {
                        // Headers alone are not a complete response: a truncated/timed-out
                        // body must not turn a 404 into independent death evidence.
                        obs.actual_request = "unknown".to_string();
                        obs.error_kind = Some(format!("CurlExitCode: {:?}", out.status.code()));
                    }
                    if matches!(obs.status_code, Some(429 | 503)) && !retry_after.trim().is_empty()
                    {
                        obs.error_kind = Some(format!("RetryAfter: {}", retry_after.trim()));
                    }
                } else if body_too_large {
                    obs.actual_request = "unknown".to_string();
                    obs.error_kind = Some("BodyTooLarge".to_string());
                } else {
                    obs.actual_request = "unknown".to_string();
                    obs.error_kind = Some(if out.status.success() {
                        "CurlMalformedJson".to_string()
                    } else {
                        format!("CurlExitCode: {:?}", out.status.code())
                    });
                }
                obs.content_length = Some(body.len() as u64);
                if !body_too_large {
                    Self::observe_html_content(&mut obs, &content_target_url, &body);
                }
            }
            Ok(Err(e)) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("CurlExecutionError: {e}"));
            }
            Err(e) => {
                obs.actual_request = "unknown".to_string();
                obs.error_kind = Some(format!("CurlExecutionError: {e}"));
            }
        }

        Ok(obs)
    }
}

#[cfg(test)]
mod tests {
    use super::CurlCliBackend;
    use crate::health::models::BackendObservation;
    use std::process::Command;

    #[test]
    fn curl_command_disables_config_and_treats_the_target_as_one_literal_url() {
        let url = "https://example.test/items[1-2]?tag={one,two}";
        let command = CurlCliBackend::build_command(
            "curl",
            url,
            std::path::Path::new("body"),
            10,
            None,
            None,
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            args[0], "-q",
            "curl must ignore its config before any other argument"
        );
        assert!(args.contains(&"--globoff"));
        assert_eq!(&args[args.len() - 2..], ["--url", url]);
    }

    #[test]
    fn curl_does_not_read_an_ambient_config_file() {
        let Some(binary) = CurlCliBackend::find_binary() else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".curlrc"),
            "synthetic-invalid-option = true\n",
        )
        .unwrap();
        let output = CurlCliBackend::build_command(
            &binary,
            "http://127.0.0.1:1/unused",
            &temp.path().join("body"),
            2,
            None,
            None,
        )
        .env("CURL_HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path())
        .env("HOME", temp.path())
        .arg("--version")
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-invalid-option"));
    }

    #[test]
    fn curl_sends_brackets_and_braces_without_expanding_the_bookmark() {
        use std::io::{BufRead, BufReader, Write};
        use std::time::{Duration, Instant};
        let Some(binary) = CurlCliBackend::find_binary() else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut request = String::new();
                        let mut reader = BufReader::new(&mut socket);
                        reader.read_line(&mut request).unwrap();
                        loop {
                            let mut header = String::new();
                            if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                                break;
                            }
                        }
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
                        return request;
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("curl fixture was not reached: {error}"),
                }
            }
        });
        let output = CurlCliBackend::build_command(
            &binary,
            &format!("http://{address}/items[1-2]?tag={{one,two}}"),
            &temp.path().join("body"),
            2,
            None,
            None,
        )
        .env("CURL_HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path())
        .env("HOME", temp.path())
        .output()
        .unwrap();
        let request = server.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(request, "GET /items[1-2]?tag={one,two} HTTP/1.1\r\n");
        let metadata = String::from_utf8(output.stdout).unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(metadata.lines().next().unwrap()).is_ok()
        );
    }

    #[test]
    fn find_binary_returns_cached_discovery_result() {
        assert_eq!(CurlCliBackend::find_binary(), CurlCliBackend::find_binary());
    }

    #[test]
    fn structured_removed_video_is_content_missing() {
        let mut observation = BackendObservation {
            content_type: Some("text/html; charset=utf-8".to_string()),
            status_code: Some(200),
            ..Default::default()
        };
        CurlCliBackend::observe_html_content(
            &mut observation,
            "https://www.youtube.com/watch?v=abcdefghijk",
            br#"<html><script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}};</script></html>"#,
        );
        assert_eq!(observation.error_kind.as_deref(), Some("ContentMissing"));
    }

    #[test]
    fn proxy_route_never_uses_local_resolution_or_no_proxy() {
        let mut command = Command::new("curl");
        CurlCliBackend::configure_route(
            &mut command,
            Some("http://127.0.0.1:7890"),
            Some("example.com:443:93.184.216.34"),
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(args, ["--proxy", "http://127.0.0.1:7890"]);
        assert!(command
            .get_envs()
            .any(|(key, value)| { key == "NO_PROXY" && value.is_none() }));
        assert!(command
            .get_envs()
            .any(|(key, value)| { key == "no_proxy" && value.is_none() }));
    }

    #[test]
    fn direct_route_pins_the_prevalidated_address() {
        let mut command = Command::new("curl");
        CurlCliBackend::configure_route(&mut command, None, Some("example.com:443:93.184.216.34"));
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            args,
            [
                "--noproxy",
                "*",
                "--resolve",
                "example.com:443:93.184.216.34"
            ]
        );
    }
}
