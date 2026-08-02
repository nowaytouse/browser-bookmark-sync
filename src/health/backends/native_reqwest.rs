//! Native Reqwest Probe Backend (Tier 1)

use crate::health::backends::ProbeBackend;
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use crate::health::security::{validate_url_parsed, validate_url_security, SecurityCheckResult};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct NativeReqwestBackend;

impl NativeReqwestBackend {
    pub fn new() -> Self {
        Self
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
        }

        builder
            .build()
            .map_err(|e| format!("Reqwest build error: {e}"))
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
        let client = Self::build_client(
            context.proxy_url.as_deref(),
            context.timeout_secs,
            context.allow_private_ips,
        )?;

        let start_time = Instant::now();

        // Try HEAD first
        let mut method = "HEAD";
        let mut response_res = client.head(&target.original_url).send().await;

        // Fallback to GET if HEAD returns 405, 403, 404, 400, or 501 (many WAFs / CDNs block HEAD)
        let mut fell_back = false;
        if let Ok(ref resp) = response_res {
            let status = resp.status().as_u16();
            if matches!(status, 405 | 403 | 404 | 400 | 501) {
                method = "GET";
                fell_back = true;
                response_res = client.get(&target.original_url).send().await;
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

                if status_code == 429 {
                    if let Some(ra) = resp.headers().get(reqwest::header::RETRY_AFTER) {
                        if let Ok(ra_str) = ra.to_str() {
                            obs.error_kind = Some(format!("RetryAfter: {ra_str}"));
                        }
                    }
                }

                if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
                    obs.content_type = ct.to_str().ok().map(|s| s.to_string());
                }
                if let Some(cl) = resp.headers().get(reqwest::header::CONTENT_LENGTH) {
                    if let Ok(l) = cl.to_str().unwrap_or("").parse::<u64>() {
                        obs.content_length = Some(l);
                    }
                }

                // If GET was performed, read body with max 256 KiB limit
                if method == "GET" || fell_back {
                    let mut buffer = Vec::new();
                    // Read up to 256 KiB
                    let max_bytes = 256 * 1024;

                    while let Ok(Some(chunk)) = resp.chunk().await {
                        let available = max_bytes - buffer.len();
                        if available == 0 {
                            break;
                        }
                        let to_take = chunk.len().min(available);
                        buffer.extend_from_slice(&chunk[..to_take]);
                    }

                    let text = String::from_utf8_lossy(&buffer);
                    obs.content_length = Some(buffer.len() as u64);

                    let preview: String = text.chars().take(500).collect();
                    obs.body_preview = Some(preview);

                    // Title, Canonical, and Body Hash extraction ONLY if Content-Type is HTML/XHTML
                    let is_html = obs.content_type.as_ref().map_or(false, |ct| {
                        let ct_lower = ct.to_lowercase();
                        ct_lower.contains("html") || ct_lower.contains("xhtml")
                    });

                    if is_html {
                        if let Some(t) = crate::health::content::extract_html_title(&text) {
                            obs.title = Some(t);
                        }
                        if let Some(c) = crate::health::content::extract_canonical_url(&text) {
                            obs.canonical_url = Some(c);
                        }
                        let hash = blake3::hash(&buffer).to_hex().to_string();
                        obs.body_hash = Some(hash);
                    }
                }
            }
            Err(err) => {
                let err_str = err.to_string();
                if err.is_timeout() {
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
