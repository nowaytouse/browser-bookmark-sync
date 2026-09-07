//! Native Reqwest Probe Backend (Tier 1)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use crate::health::security::{
    is_ip_allowed, validate_url_parsed, validate_url_security, SecurityCheckResult,
};
use crate::health::targets::ProbeScope;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct NativeReqwestBackend {
    client: Option<Client>,
    client_config: Option<(Option<String>, u64, bool)>,
}

#[derive(Debug)]
struct DnsSecurityError(String);

impl fmt::Display for DnsSecurityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DnsSecurityError {}

fn validate_resolved_addresses(
    addresses: &[SocketAddr],
    allow_private: bool,
) -> Result<(), String> {
    for address in addresses {
        if let Err(reason) = is_ip_allowed(address.ip(), allow_private) {
            return Err(format!("SecurityBlocked: {reason}"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct SecureSystemResolver {
    allow_private: bool,
}

impl reqwest::dns::Resolve for SecureSystemResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let allow_private = self.allow_private;
        Box::pin(async move {
            let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?
                .collect();
            validate_resolved_addresses(&addresses, allow_private)
                .map_err(|reason| Box::new(DnsSecurityError(reason)))?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl NativeReqwestBackend {
    pub const fn new() -> Self {
        Self {
            client: None,
            client_config: None,
        }
    }

    pub fn configured(context: &ProbeContext) -> Result<Self, String> {
        let config = (
            context.proxy_url.clone(),
            context.timeout_secs,
            context.allow_private_ips,
        );
        let client = Self::build_client(config.0.as_deref(), config.1, config.2)?;
        Ok(Self {
            client: Some(client),
            client_config: Some(config),
        })
    }

    fn build_client(
        proxy_url: Option<&str>,
        timeout_secs: u64,
        allow_private: bool,
    ) -> Result<Client, String> {
        let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("Too many redirects");
            }
            if let SecurityCheckResult::Denied(reason) =
                validate_url_parsed(attempt.url(), allow_private)
            {
                return attempt.error(format!("SecurityBlocked: {reason}"));
            }
            attempt.follow()
        });

        let mut builder = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .redirect(redirect_policy);

        if let Some(proxy) = proxy_url {
            let p = reqwest::Proxy::all(proxy).map_err(|e| format!("Proxy config error: {e}"))?;
            builder = builder.proxy(p);
        } else {
            builder = builder.dns_resolver(Arc::new(SecureSystemResolver { allow_private }));
        }

        builder
            .build()
            .map_err(|e| format!("Reqwest build error: {e}"))
    }

    fn client_for(&self, context: &ProbeContext) -> Result<Client, String> {
        let config = (
            context.proxy_url.clone(),
            context.timeout_secs,
            context.allow_private_ips,
        );
        if self.client_config.as_ref() == Some(&config) {
            if let Some(client) = &self.client {
                return Ok(client.clone());
            }
        }
        Self::build_client(config.0.as_deref(), config.1, config.2)
    }

    /// Check if initial deep URL was redirected to root homepage
    fn is_redirected_to_homepage(initial_url_str: &str, final_url_str: &str) -> bool {
        if let (Ok(u1), Ok(u2)) = (
            reqwest::Url::parse(initial_url_str),
            reqwest::Url::parse(final_url_str),
        ) {
            let path1 = u1.path().trim_matches('/');
            let path2 = u2.path().trim_matches('/');

            // Initial URL had a deep path (e.g. /docs/v1/api), but final URL returned / or index.html
            if !path1.is_empty()
                && (path2.is_empty() || path2 == "index.html" || path2 == "index.php")
            {
                return true;
            }
        }
        false
    }
}

impl Default for NativeReqwestBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProbeBackend for NativeReqwestBackend {
    fn backend_id(&self) -> &'static str {
        "native_reqwest"
    }

    async fn probe(
        &self,
        target: &ProbeTarget,
        context: &ProbeContext,
    ) -> Result<BackendObservation, String> {
        let started_at = Utc::now();

        // Tier 0 Security Pre-check
        if let SecurityCheckResult::Denied(reason) =
            validate_url_security(&target.original_url, context.allow_private_ips)
        {
            let mut obs = BackendObservation::default();
            obs.backend = self.backend_id().to_string();
            obs.backend_version = env!("CARGO_PKG_VERSION").to_string();
            obs.run_id = context.run_id.clone();
            obs.check_id = context.check_id.clone();
            obs.started_at = started_at;
            obs.finished_at = Utc::now();
            obs.actual_request = "no".to_string();
            obs.initial_url = target.original_url.clone();
            obs.error_kind = Some(format!("SecurityBlocked: {reason}"));
            return Ok(obs);
        }

        let route = if context.proxy_url.is_some() {
            "proxy"
        } else {
            "direct"
        };
        let client = self.client_for(context)?;

        let start_time = Instant::now();

        // Content checks need the response body, so issue one GET directly.  The old HEAD→GET
        // sequence doubled requests for every saved page and made long-tail scans needlessly
        // slow.  Site checks stay HEAD-first and only fall back when a server rejects HEAD.
        let mut method = if context.probe_scope == ProbeScope::Content {
            "GET"
        } else {
            "HEAD"
        };
        let mut response_res = if context.probe_scope == ProbeScope::Content {
            client.get(&target.original_url).send().await
        } else {
            client.head(&target.original_url).send().await
        };

        let mut fell_back = false;
        if context.probe_scope == ProbeScope::Site {
            if let Ok(ref resp) = response_res {
                let status = resp.status().as_u16();
                if matches!(status, 400 | 403 | 404 | 405 | 501) {
                    method = "GET";
                    fell_back = true;
                    response_res = client.get(&target.original_url).send().await;
                }
            }
        }

        let latency_ms = start_time.elapsed().as_millis() as u64;
        let finished_at = Utc::now();

        let mut obs = BackendObservation::default();
        obs.backend = self.backend_id().to_string();
        obs.backend_version = env!("CARGO_PKG_VERSION").to_string();
        obs.run_id = context.run_id.clone();
        obs.check_id = context.check_id.clone();
        obs.route = route.to_string();
        obs.started_at = started_at;
        obs.finished_at = finished_at;
        obs.actual_request = "yes".to_string();
        obs.request_method = method.to_string();
        obs.initial_url = target.original_url.clone();
        obs.response_time_ms = latency_ms;

        match response_res {
            Ok(mut resp) => {
                let status_code = resp.status().as_u16();
                obs.status_code = Some(status_code);

                let final_url_str = resp.url().to_string();
                if final_url_str != target.original_url {
                    obs.final_url = Some(final_url_str.clone());
                    obs.redirect_count = 1;

                    if Self::is_redirected_to_homepage(&target.original_url, &final_url_str) {
                        obs.error_kind = Some("RedirectedToHomepage".to_string());
                    }
                }

                if matches!(status_code, 429 | 503) {
                    if let Some(ra) = resp.headers().get(reqwest::header::RETRY_AFTER) {
                        if let Ok(ra_str) = ra.to_str() {
                            obs.error_kind = Some(format!("RetryAfter: {ra_str}"));
                        }
                    }
                }

                if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
                    obs.content_type = ct.to_str().ok().map(ToString::to_string);
                }
                if let Some(cl) = resp.headers().get(reqwest::header::CONTENT_LENGTH) {
                    if let Ok(l) = cl.to_str().unwrap_or("").parse::<u64>() {
                        obs.content_length = Some(l);
                    }
                }

                let declared_html = obs.content_type.as_ref().is_some_and(|content_type| {
                    let lower = content_type.to_lowercase();
                    lower.contains("html") || lower.contains("xhtml")
                });
                if obs.content_type.is_some() && !declared_html {
                    return Ok(obs);
                }

                // If GET was performed, read body with max 256 KiB limit
                if method == "GET" || fell_back {
                    let mut buffer = Vec::new();
                    let max_bytes = 256 * 1024;
                    let mut body_read_error = None;

                    while buffer.len() < max_bytes {
                        match resp.chunk().await {
                            Ok(Some(chunk)) => {
                                let available = max_bytes - buffer.len();
                                let to_take = chunk.len().min(available);
                                buffer.extend_from_slice(&chunk[..to_take]);
                                if to_take < chunk.len() {
                                    break;
                                }
                            }
                            Ok(None) => break,
                            Err(error) => {
                                body_read_error = Some(format!("BodyReadError: {error}"));
                                break;
                            }
                        }
                    }

                    if obs.content_length.is_none() {
                        obs.content_length = Some(buffer.len() as u64);
                    }
                    if let Some(issue) = body_read_error {
                        obs.actual_request = "unknown".to_string();
                        obs.error_kind = Some(issue);
                        return Ok(obs);
                    }
                    if buffer.is_empty() {
                        return Ok(obs);
                    }

                    // Title, Canonical, and Body Hash extraction ONLY if Content-Type is HTML/XHTML
                    if declared_html {
                        let text = String::from_utf8_lossy(&buffer);
                        if let Some(t) = crate::health::content::extract_html_title(&text) {
                            obs.title = Some(t);
                        }
                        if let Some(c) = crate::health::content::extract_canonical_url(&text) {
                            obs.canonical_url = Some(c);
                        }
                        let hash = blake3::hash(&buffer).to_hex().to_string();
                        obs.body_hash = Some(hash);
                        if let Some(visible_text) =
                            crate::health::content::extract_content_text(&text)
                        {
                            obs.body_preview = Some(visible_text);
                        }
                        if obs.error_kind.is_none() {
                            if crate::health::content::is_parked_domain(
                                obs.title.as_deref(),
                                obs.body_preview.as_deref(),
                            ) {
                                obs.error_kind = Some("ParkedDomainCandidate".to_string());
                            } else if crate::health::content::is_deep_content_url(
                                &target.original_url,
                            ) && crate::health::content::is_structured_content_missing(
                                &text,
                            ) {
                                obs.error_kind = Some("ContentMissing".to_string());
                            } else if crate::health::content::is_soft_404(
                                obs.title.as_deref(),
                                obs.body_preview.as_deref(),
                            ) {
                                obs.error_kind = Some("Soft404Candidate".to_string());
                            }
                        }
                    }
                }
            }
            Err(err) => {
                let err_str = err.to_string();
                if let Some(index) = err_str.find("SecurityBlocked:") {
                    obs.error_kind = Some(err_str[index..].to_string());
                } else if err.is_timeout() {
                    obs.error_kind = Some("Timeout".to_string());
                } else if err.is_connect() {
                    if err_str.to_lowercase().contains("dns")
                        || err_str.to_lowercase().contains("no such host")
                    {
                        obs.error_kind = Some("DnsError".to_string());
                    } else {
                        obs.error_kind = Some("ConnectionError".to_string());
                    }
                } else {
                    obs.error_kind = Some(format!("HttpError: {err_str}"));
                }
            }
        }

        Ok(obs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn resolved_private_or_mixed_addresses_are_blocked() {
        let private = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 80);
        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80);
        let public = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)), 80);

        assert!(validate_resolved_addresses(&[private], false)
            .unwrap_err()
            .starts_with("SecurityBlocked:"));
        assert!(validate_resolved_addresses(&[loopback], false)
            .unwrap_err()
            .starts_with("SecurityBlocked:"));
        assert!(validate_resolved_addresses(&[public], false).is_ok());
        assert!(validate_resolved_addresses(&[public, private], false)
            .unwrap_err()
            .starts_with("SecurityBlocked:"));
        assert!(validate_resolved_addresses(&[private], true).is_ok());
    }

    async fn probe_html(path: &str, html: &str) -> BackendObservation {
        probe_html_with_scope(path, html, ProbeScope::Content).await
    }

    async fn probe_html_with_scope(
        path: &str,
        html: &str,
        probe_scope: ProbeScope,
    ) -> BackendObservation {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = html.to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let size = socket.read(&mut request).await.unwrap();
            let is_head = request[..size].starts_with(b"HEAD ");
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            if !is_head {
                let _ = socket.write_all(body.as_bytes()).await;
            }
        });

        let url = format!("http://{address}{path}");
        let target = ProbeTarget::from_url("test-bookmark", &url).unwrap();
        let context = ProbeContext {
            run_id: "test-run".to_string(),
            check_id: "test-check".to_string(),
            proxy_url: None,
            timeout_secs: 5,
            slow_mode: false,
            probe_scope,
            dns_policy_hash: "test-dns-policy".to_string(),
            allow_private_ips: true,
            history_db_path: None,
            secondary_backends: Vec::new(),
            artifact_dir: None,
        };
        let observation = NativeReqwestBackend::new()
            .probe(&target, &context)
            .await
            .unwrap();
        server.await.unwrap();
        observation
    }

    #[tokio::test]
    async fn html_200_still_detects_parked_site_and_removed_video() {
        let parked = probe_html("/", "<html><title>Domain for Sale</title></html>").await;
        assert_eq!(parked.request_method, "GET");
        assert_eq!(parked.error_kind.as_deref(), Some("ParkedDomainCandidate"));

        let removed_video = probe_html(
            "/watch?v=gone",
            r#"<html><script>{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}}</script></html>"#,
        )
        .await;
        assert_eq!(removed_video.request_method, "GET");
        assert_eq!(removed_video.error_kind.as_deref(), Some("ContentMissing"));
    }

    #[tokio::test]
    async fn site_probe_uses_one_head_request_for_a_live_html_homepage() {
        let site =
            probe_html_with_scope("/", "<html><title>Live</title></html>", ProbeScope::Site).await;
        assert_eq!(site.request_method, "HEAD");
        assert!(site.body_hash.is_none());
    }

    #[tokio::test]
    async fn empty_html_is_not_success_but_an_oversized_prefix_is_usable() {
        let empty = probe_html("/empty", "").await;
        assert_eq!(empty.content_length, Some(0));
        assert!(empty.body_hash.is_none());

        let oversized_html = format!("<html>{}</html>", "x".repeat(300 * 1024));
        let oversized = probe_html("/large", &oversized_html).await;
        assert_eq!(oversized.actual_request, "yes");
        assert!(oversized.error_kind.is_none());
        assert!(oversized.body_hash.is_some());
    }
}
