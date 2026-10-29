//! Native Reqwest Probe Backend (Tier 1)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use crate::health::scheduler::{HostScheduler, RequestPermit, SchedulerConfig};
use crate::health::security::{
    is_ip_allowed, validate_url_parsed, validate_url_security, SecurityCheckResult,
};
use crate::health::targets::ProbeScope;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::{Client, Method, Response};
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct NativeReqwestBackend {
    client: Option<Client>,
    client_config: Option<(Option<String>, u64, bool)>,
    schedulers: [Arc<OnceLock<HostScheduler>>; 2],
}

// Field order ensures the response is disposed before its host/global permission.
struct ScheduledResponse {
    response: Response,
    _permit: RequestPermit,
}

enum RequestError {
    Transport(reqwest::Error),
    Policy(String),
}

#[derive(Default)]
struct RequestTrace {
    requests_sent: usize,
    first_send: Option<(chrono::DateTime<Utc>, Instant)>,
    redirects: Vec<String>,
}

impl From<reqwest::Error> for RequestError {
    fn from(error: reqwest::Error) -> Self {
        Self::Transport(error)
    }
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
    pub fn new() -> Self {
        Self {
            client: None,
            client_config: None,
            schedulers: std::array::from_fn(|_| Arc::new(OnceLock::new())),
        }
    }

    pub fn configured(context: &ProbeContext) -> Result<Self, String> {
        Self::configured_with_scheduler(context, Self::default_scheduler(context.slow_mode))
    }

    pub fn configured_with_scheduler(
        context: &ProbeContext,
        scheduler: HostScheduler,
    ) -> Result<Self, String> {
        let config = (
            context.proxy_url.clone(),
            context.timeout_secs,
            context.allow_private_ips,
        );
        let client = Self::build_client(config.0.as_deref(), config.1, config.2)?;
        let schedulers = std::array::from_fn(|index| {
            Arc::new(if index == usize::from(context.slow_mode) {
                OnceLock::from(scheduler.clone())
            } else {
                OnceLock::new()
            })
        });
        Ok(Self {
            client: Some(client),
            client_config: Some(config),
            schedulers,
        })
    }

    fn default_scheduler(slow: bool) -> HostScheduler {
        HostScheduler::new(if slow {
            SchedulerConfig::slow_mode()
        } else {
            SchedulerConfig::default()
        })
    }

    fn scheduler_for(&self, context: &ProbeContext) -> &HostScheduler {
        self.schedulers[usize::from(context.slow_mode)]
            .get_or_init(|| Self::default_scheduler(context.slow_mode))
    }

    fn build_client(
        proxy_url: Option<&str>,
        timeout_secs: u64,
        allow_private: bool,
    ) -> Result<Client, String> {
        let mut builder = Client::builder()
            // Route evidence must describe the actual connection, not inherited proxy settings.
            .no_proxy()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            // Every redirect and explicit retry must re-enter the shared scheduler.
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());

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

    async fn send_chain(
        &self,
        client: &Client,
        target: &ProbeTarget,
        context: &ProbeContext,
        method: Method,
        trace: &mut RequestTrace,
    ) -> Result<ScheduledResponse, RequestError> {
        let scheduler = self.scheduler_for(context);
        let mut request = client.request(method, &target.original_url).build()?;
        let mut deadline = None;
        let mut followed = 0;
        loop {
            if let SecurityCheckResult::Denied(reason) =
                validate_url_parsed(request.url(), context.allow_private_ips)
            {
                return Err(RequestError::Policy(format!("SecurityBlocked: {reason}")));
            }
            let current = ProbeTarget::from_url("request", request.url().as_str())
                .map_err(RequestError::Policy)?;
            let mut keys = vec![current.host.as_str()];
            if current.registrable_domain != current.host {
                keys.push(current.registrable_domain.as_str());
            }
            let acquire = scheduler.acquire_request(&keys);
            let mut permit = if let Some(at) = deadline {
                tokio::time::timeout_at(at, acquire)
                    .await
                    .map_err(|_| RequestError::Policy("Timeout".into()))?
            } else {
                acquire.await
            }
            .map_err(|error| RequestError::Policy(format!("SchedulingError: {error}")))?;
            let at = match deadline {
                Some(at) => at,
                None => tokio::time::Instant::now()
                    .checked_add(Duration::from_secs(context.timeout_secs))
                    .ok_or_else(|| RequestError::Policy("Invalid request timeout".into()))?,
            };
            deadline = Some(at);
            let remaining = at.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(RequestError::Policy("Timeout".into()));
            }
            *request.timeout_mut() = Some(remaining);
            let outgoing = request
                .try_clone()
                .ok_or_else(|| RequestError::Policy("Cannot repeat request safely".into()))?;
            if trace.requests_sent == 0 {
                trace.first_send = Some((Utc::now(), Instant::now()));
            }
            trace.requests_sent += 1;
            if followed > 0 {
                trace.redirects.push(request.url().to_string());
            }
            let response = client.execute(outgoing).await?;
            let status = response.status().as_u16();
            let is_redirect = matches!(status, 301 | 302 | 303 | 307 | 308);
            let default_wait = if context.slow_mode { 60 } else { 30 };
            let server_wait = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(|value| HostScheduler::parse_retry_after(value, default_wait));
            if let Some(wait) = server_wait
                .or_else(|| matches!(status, 429 | 503).then(|| Duration::from_secs(default_wait)))
            {
                permit.set_cooldown(wait);
            }
            let next = is_redirect
                .then(|| response.headers().get(reqwest::header::LOCATION))
                .flatten()
                .and_then(|location| location.to_str().ok())
                .and_then(|location| request.url().join(location).ok());
            let Some(mut next) = next else {
                return Ok(ScheduledResponse {
                    response,
                    _permit: permit,
                });
            };
            // Preserve the old policy's limit: attempt.previous().len() >= 10.
            if followed + 1 >= 10 {
                return Err(RequestError::Policy("HttpError: Too many redirects".into()));
            }
            if let SecurityCheckResult::Denied(reason) =
                validate_url_parsed(&next, context.allow_private_ips)
            {
                return Err(RequestError::Policy(format!("SecurityBlocked: {reason}")));
            }
            // Do not synthesize new BasicAuth from untrusted Location userinfo.
            next.set_username("")
                .map_err(|_| RequestError::Policy("Invalid redirect userinfo".into()))?;
            next.set_password(None)
                .map_err(|_| RequestError::Policy("Invalid redirect userinfo".into()))?;
            let previous = request.url().clone();
            if next.origin() != previous.origin() {
                for name in [
                    "authorization",
                    "cookie",
                    "cookie2",
                    "proxy-authorization",
                    "www-authenticate",
                ] {
                    request.headers_mut().remove(name);
                }
            }
            request.headers_mut().remove(reqwest::header::REFERER);
            if !(previous.scheme() == "https" && next.scheme() == "http") {
                let mut referer = previous.clone();
                referer
                    .set_username("")
                    .map_err(|_| RequestError::Policy("Invalid referrer userinfo".into()))?;
                referer
                    .set_password(None)
                    .map_err(|_| RequestError::Policy("Invalid referrer userinfo".into()))?;
                referer.set_fragment(None);
                if let Ok(value) = reqwest::header::HeaderValue::from_str(referer.as_str()) {
                    request
                        .headers_mut()
                        .insert(reqwest::header::REFERER, value);
                }
            }
            // Drop the previous response before releasing its accounting or waiting.
            drop(response);
            drop(permit);
            if let Some(wait) = server_wait {
                // A server delay applies before following even to a different host.
                // Never overflow a timer or reset the chain's transport deadline.
                if wait >= at.saturating_duration_since(tokio::time::Instant::now()) {
                    return Err(RequestError::Policy("Timeout".into()));
                }
                tokio::time::sleep(wait).await;
            }
            *request.url_mut() = next.clone();
            followed += 1;
        }
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
                && u2.query().is_none()
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

        // Content checks need the response body, so issue one GET directly.  The old HEAD→GET
        // sequence doubled requests for every saved page and made long-tail scans needlessly
        // slow.  Site checks stay HEAD-first and only fall back when a server rejects HEAD.
        let mut method = if context.probe_scope == ProbeScope::Content {
            "GET"
        } else {
            "HEAD"
        };
        let mut trace = RequestTrace::default();
        let first_method = if context.probe_scope == ProbeScope::Content {
            Method::GET
        } else {
            Method::HEAD
        };
        let mut response_res = self
            .send_chain(&client, target, context, first_method, &mut trace)
            .await;

        let mut fell_back = false;
        let needs_fallback = context.probe_scope == ProbeScope::Site
            && response_res.as_ref().is_ok_and(|result| {
                matches!(
                    result.response.status().as_u16(),
                    400 | 403 | 404 | 405 | 501
                )
            });
        if needs_fallback {
            method = "GET";
            fell_back = true;
            drop(response_res);
            response_res = self
                .send_chain(&client, target, context, Method::GET, &mut trace)
                .await;
        }

        let latency_ms = trace
            .first_send
            .as_ref()
            .map_or(0, |(_, start)| start.elapsed().as_millis() as u64);
        let finished_at = Utc::now();

        let mut obs = BackendObservation::default();
        obs.backend = self.backend_id().to_string();
        obs.backend_version = env!("CARGO_PKG_VERSION").to_string();
        obs.run_id = context.run_id.clone();
        obs.check_id = context.check_id.clone();
        obs.route = route.to_string();
        obs.started_at = trace.first_send.as_ref().map_or(started_at, |(at, _)| *at);
        obs.finished_at = finished_at;
        obs.actual_request = if trace.requests_sent == 0 {
            "no"
        } else {
            "yes"
        }
        .to_string();
        obs.request_method = method.to_string();
        obs.initial_url = target.original_url.clone();
        obs.response_time_ms = latency_ms;
        obs.redirect_count = trace.redirects.len();
        obs.redirect_chain = trace.redirects;

        match response_res {
            Ok(mut scheduled) => {
                let resp = &mut scheduled.response;
                let status_code = resp.status().as_u16();
                obs.status_code = Some(status_code);

                let final_url_str = resp.url().to_string();
                if final_url_str != target.original_url {
                    obs.final_url = Some(final_url_str.clone());

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

                    loop {
                        match resp.chunk().await {
                            Ok(Some(chunk)) => {
                                let available = max_bytes - buffer.len();
                                let to_take = chunk.len().min(available);
                                buffer.extend_from_slice(&chunk[..to_take]);
                                if to_take < chunk.len() {
                                    body_read_error = Some("BodyTooLarge".to_string());
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
                            } else if let Some(issue) =
                                crate::health::content::structured_content_error(
                                    obs.final_url.as_deref().unwrap_or(&target.original_url),
                                    &text,
                                )
                            {
                                obs.error_kind = Some(issue.to_string());
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
            Err(RequestError::Policy(reason)) => {
                obs.error_kind = Some(reason);
            }
            Err(RequestError::Transport(err)) => {
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
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:80".parse().unwrap();
        assert!(validate_resolved_addresses(&[public, mapped], false)
            .unwrap_err()
            .starts_with("SecurityBlocked:"));
        assert!(validate_resolved_addresses(&[mapped], true).is_ok());
    }

    #[test]
    fn explicit_routes_ignore_inherited_proxies() {
        // A child test process isolates environment changes from parallel tests.
        for route in ["direct", "proxy"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let server = std::thread::spawn(move || {
                use std::io::{BufRead, BufReader, Write};
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            // macOS accepted sockets inherit the listener's nonblocking flag.
                            socket.set_nonblocking(false).unwrap();
                            socket
                                .set_read_timeout(Some(Duration::from_secs(10)))
                                .unwrap();
                            // TCP may split the request line; share the child's timeout budget.
                            let mut request = String::new();
                            BufReader::new(&mut socket).read_line(&mut request).unwrap();
                            if route == "proxy" {
                                assert!(request.starts_with("GET http://127.0.0.1:9/route "));
                            } else {
                                assert!(request.starts_with("GET /route "));
                            }
                            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
                            break;
                        }
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => panic!("route fixture was not reached: {error}"),
                    }
                }
            });
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "health::backends::native_reqwest::tests::route_fixture_child",
                    "--nocapture",
                ])
                .env("BOOKMARK_ROUTE_FIXTURE", route)
                .env("BOOKMARK_ROUTE_ADDRESS", address.to_string())
                .env("HTTP_PROXY", "http://127.0.0.1:1")
                .env("http_proxy", "http://127.0.0.1:1")
                .env("HTTPS_PROXY", "http://127.0.0.1:1")
                .env("https_proxy", "http://127.0.0.1:1")
                .env("ALL_PROXY", "http://127.0.0.1:1")
                .env("all_proxy", "http://127.0.0.1:1")
                .env("NO_PROXY", "")
                .env("no_proxy", "")
                .output()
                .unwrap();
            let fixture_result = server.join();
            assert!(
                result.status.success(),
                "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            fixture_result.unwrap();
            assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
        }
    }

    #[tokio::test]
    async fn route_fixture_child() {
        let Ok(route) = std::env::var("BOOKMARK_ROUTE_FIXTURE") else {
            return;
        };
        let address = std::env::var("BOOKMARK_ROUTE_ADDRESS").unwrap();
        let proxy = (route == "proxy").then(|| format!("http://{address}"));
        let target = if proxy.is_some() {
            "http://127.0.0.1:9/route".into()
        } else {
            format!("http://{address}/route")
        };
        let client = NativeReqwestBackend::build_client(proxy.as_deref(), 10, true).unwrap();
        assert_eq!(
            client
                .get(target)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
    }

    async fn probe_html(path: &str, html: &str) -> BackendObservation {
        probe_html_with_scope(path, html, ProbeScope::Content).await
    }

    async fn probe_html_with_scope(
        path: &str,
        html: &str,
        probe_scope: ProbeScope,
    ) -> BackendObservation {
        probe_html_response(path, html, probe_scope, 200, Some(html.len())).await
    }

    async fn probe_html_response(
        path: &str,
        html: &str,
        probe_scope: ProbeScope,
        status: u16,
        declared_length: Option<usize>,
    ) -> BackendObservation {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = html.to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let size = socket.read(&mut request).await.unwrap();
            let is_head = request[..size].starts_with(b"HEAD ");
            let length_header = declared_length
                .map(|length| format!("Content-Length: {length}\r\n"))
                .unwrap_or_default();
            let headers = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: text/html; charset=utf-8\r\n{length_header}Connection: close\r\n\r\n"
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            if !is_head {
                let _ = socket.write_all(body.as_bytes()).await;
            }
        });

        let url = format!("http://{address}{path}");
        let observation = probe_local_target(&url, probe_scope).await;
        tokio::time::timeout(Duration::from_secs(6), server)
            .await
            .unwrap()
            .unwrap();
        observation
    }

    async fn probe_local_target(url: &str, probe_scope: ProbeScope) -> BackendObservation {
        let target = ProbeTarget::from_url("test-bookmark", url).unwrap();
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
        NativeReqwestBackend::new()
            .probe(&target, &context)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn html_200_still_detects_parked_site_and_removed_video() {
        let parked = probe_html("/", "<html><title>Domain for Sale</title></html>").await;
        assert_eq!(parked.request_method, "GET");
        assert_eq!(parked.error_kind.as_deref(), Some("ParkedDomainCandidate"));

        let removed_video = probe_player_page(
            &NativeReqwestBackend::new(),
            r#"<html><script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}};</script></html>"#,
        )
        .await;
        assert_eq!(removed_video.request_method, "GET");
        assert_eq!(removed_video.error_kind.as_deref(), Some("ContentMissing"));
    }

    // A loopback HTTP proxy serves a provider URL without DNS or internet access.
    async fn probe_player_page(backend: &dyn ProbeBackend, html: &str) -> BackendObservation {
        use tokio::io::AsyncBufReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = html.to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(&mut socket);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert_eq!(
                line,
                "GET http://www.youtube.com/watch?v=abcdefghijk HTTP/1.1\r\n"
            );
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            drop(reader);
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let target =
            ProbeTarget::from_url("video", "http://www.youtube.com/watch?v=abcdefghijk").unwrap();
        let context = ProbeContext {
            run_id: "player-fixture".into(),
            check_id: "player-fixture".into(),
            proxy_url: Some(format!("http://{address}")),
            timeout_secs: 5,
            slow_mode: false,
            probe_scope: ProbeScope::Content,
            dns_policy_hash: "synthetic".into(),
            allow_private_ips: true,
            history_db_path: None,
            secondary_backends: Vec::new(),
            artifact_dir: None,
        };
        let observation = backend.probe(&target, &context).await.unwrap();
        tokio::time::timeout(Duration::from_secs(6), server)
            .await
            .unwrap()
            .unwrap();
        observation
    }

    #[tokio::test]
    async fn accuracy_quoted_player_error_is_not_page_deletion_evidence() {
        let article = probe_html(
            "/guides/player-errors",
            r#"<html><title>Player response guide</title><h1>Understanding player responses</h1><pre>{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}}</pre></html>"#,
        )
        .await;
        assert_eq!(article.actual_request, "yes");
        assert!(article.body_hash.is_some());
        assert_eq!(
            article.error_kind, None,
            "normal article must not supply missing-content evidence"
        );
        assert_eq!(
            crate::health::state_machine::StateMachine::compute_single_run_verdict(&[article]),
            crate::health::models::HealthVerdict::Live,
            "quoted player JSON is article content, not evidence that this article was removed"
        );
    }

    #[tokio::test]
    async fn accuracy_player_rules_agree_across_native_and_curl_probes() {
        use crate::health::backends::curl::CurlCliBackend;
        use crate::health::models::HealthVerdict;
        let native = NativeReqwestBackend::new();
        let curl = CurlCliBackend::new();
        let mut backends: Vec<&dyn ProbeBackend> = vec![&native];
        if CurlCliBackend::find_binary().is_some() {
            backends.push(&curl);
        }
        for backend in backends {
            for (status, reason, expected_error, expected_verdict) in [
                (
                    "ERROR",
                    "Video has been removed",
                    "ContentMissing",
                    HealthVerdict::ProbableDead,
                ),
                (
                    "UNPLAYABLE",
                    "Video unavailable",
                    "StructuredContentUncertain",
                    HealthVerdict::NeedsReview,
                ),
                (
                    "LOGIN_REQUIRED",
                    "This video is private",
                    "StructuredContentUncertain",
                    HealthVerdict::NeedsReview,
                ),
            ] {
                let html = format!(
                    r#"<title>Player page</title><script>var ytInitialPlayerResponse = {{"playabilityStatus":{{"status":"{status}","reason":"{reason}"}}}};</script>"#
                );
                let observation = probe_player_page(backend, &html).await;
                assert_eq!(observation.actual_request, "yes");
                assert_eq!(observation.error_kind.as_deref(), Some(expected_error));
                assert_eq!(
                    crate::health::state_machine::StateMachine::compute_single_run_verdict(&[
                        observation
                    ]),
                    expected_verdict
                );
            }
        }
    }

    #[tokio::test]
    async fn site_probe_uses_one_head_request_for_a_live_html_homepage() {
        let site =
            probe_html_with_scope("/", "<html><title>Live</title></html>", ProbeScope::Site).await;
        assert_eq!(site.request_method, "HEAD");
        assert!(site.body_hash.is_none());
    }

    #[tokio::test]
    async fn accuracy_incomplete_html_is_not_full_content_evidence() {
        let empty = probe_html("/empty", "").await;
        assert_eq!(empty.content_length, Some(0));
        assert!(empty.body_hash.is_none());

        let limit = 256 * 1024;
        for status in [200, 404, 410] {
            for declared in [None, Some(limit + 1)] {
                let oversized = probe_html_response(
                    "/large",
                    &"x".repeat(limit + 1),
                    ProbeScope::Content,
                    status,
                    declared,
                )
                .await;
                assert_eq!(oversized.actual_request, "unknown");
                assert_eq!(oversized.error_kind.as_deref(), Some("BodyTooLarge"));
                assert!(oversized.body_hash.is_none());
                assert!(oversized.body_preview.is_none());
            }
        }
        for declared in [None, Some(limit)] {
            let exact = probe_html_response(
                "/exact",
                &"x".repeat(limit),
                ProbeScope::Content,
                200,
                declared,
            )
            .await;
            assert_eq!(exact.actual_request, "yes");
            assert!(exact.error_kind.is_none());
            assert!(exact.body_hash.is_some());
        }
        let truncated = probe_html_response(
            "/truncated",
            &"x".repeat(limit),
            ProbeScope::Content,
            404,
            Some(limit + 1),
        )
        .await;
        assert_eq!(truncated.actual_request, "unknown");
        assert!(truncated
            .error_kind
            .as_deref()
            .unwrap()
            .starts_with("BodyReadError:"));
        assert!(truncated.body_hash.is_none());
    }

    #[tokio::test]
    async fn accuracy_query_routed_redirect_preserves_the_article() {
        use tokio::io::AsyncBufReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for redirect in [true, false] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut reader = tokio::io::BufReader::new(&mut socket);
                let mut request = String::new();
                assert!(reader.read_line(&mut request).await.unwrap() > 0);
                let expected_path = if redirect {
                    "/articles/42"
                } else {
                    "/index.php?article=42"
                };
                assert_eq!(request, format!("GET {expected_path} HTTP/1.1\r\n"));
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).await.unwrap() == 0 || header == "\r\n" {
                        break;
                    }
                }
                drop(reader);
                let response = if redirect {
                    "HTTP/1.1 301 Moved\r\nLocation: /index.php?article=42\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 25\r\nConnection: close\r\n\r\n<title>Article 42</title>"
                };
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let observation = probe_local_target(
            &format!("http://{address}/articles/42"),
            ProbeScope::Content,
        )
        .await;
        tokio::time::timeout(Duration::from_secs(6), server)
            .await
            .unwrap()
            .unwrap();
        assert!(
            observation.error_kind.is_none(),
            "{:?}",
            observation.error_kind
        );
        assert_eq!(observation.title.as_deref(), Some("Article 42"));
        assert_eq!(
            crate::health::state_machine::StateMachine::compute_single_run_verdict(&[observation]),
            crate::health::models::HealthVerdict::Live
        );
    }
}
