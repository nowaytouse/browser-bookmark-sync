//! Unified Data Models for Health Probe System

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Final health verdict assigned to a URL after multi-backend probe and historical analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthVerdict {
    /// Confirmed working and reachable URL
    Live,
    /// Previously failing but recovered in current run
    Recovered,
    /// 403 / 429 / WAF / Cloudflare challenge protected URL
    Protected,
    /// Transient failure (5xx, timeout, rate limit, network reset)
    TemporaryFailure,
    /// High probability of dead URL (e.g. single run 404/DNS error)
    ProbableDead,
    /// Confirmed dead URL (meets 3 runs over 7 days criteria across multiple tools)
    ConfirmedDead,
    /// Ambiguous / contradicting results requiring human inspection
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StrongFailureKind {
    ConfirmedNxDomain,
    Http404,
    Http410,
    Http451,
    RedirectedToHomepage,
    ContentMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualifyingRunEvidence {
    pub run_id: String,
    pub completed_at: DateTime<Utc>,
    pub strong_failure_at: DateTime<Utc>,
    pub verifier_families: std::collections::BTreeSet<String>,
    pub strong_failure_kinds: std::collections::BTreeSet<StrongFailureKind>,
}

impl HealthVerdict {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Recovered => "recovered",
            Self::Protected => "protected",
            Self::TemporaryFailure => "temporary_failure",
            Self::ProbableDead => "probable_dead",
            Self::ConfirmedDead => "confirmed_dead",
            Self::NeedsReview => "needs_review",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "live" => Some(Self::Live),
            "recovered" => Some(Self::Recovered),
            "protected" => Some(Self::Protected),
            "temporary_failure" => Some(Self::TemporaryFailure),
            "probable_dead" => Some(Self::ProbableDead),
            "confirmed_dead" => Some(Self::ConfirmedDead),
            "needs_review" => Some(Self::NeedsReview),
            _ => None,
        }
    }
}

/// Target URL metadata to be probed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeTarget {
    pub bookmark_id: String,
    pub original_url: String,
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub registrable_domain: String,
    pub original_title: Option<String>,
}

impl ProbeTarget {
    pub fn from_url(bookmark_id: impl Into<String>, url_str: &str) -> Result<Self, String> {
        let parsed = reqwest::Url::parse(url_str).map_err(|e| format!("Invalid URL: {e}"))?;
        let scheme = parsed.scheme().to_lowercase();
        let host = parsed.host_str().unwrap_or("").to_string();
        let port = parsed.port();

        // Extract registrable domain
        let registrable_domain = if host.is_empty() {
            String::new()
        } else {
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() >= 2 {
                parts[parts.len() - 2..].join(".")
            } else {
                host.clone()
            }
        };

        Ok(Self {
            bookmark_id: bookmark_id.into(),
            original_url: url_str.to_string(),
            scheme,
            host,
            port,
            registrable_domain,
            original_title: None,
        })
    }
}

/// Normalized observation output from a single probe backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendObservation {
    pub backend: String,
    pub backend_version: String,
    pub run_id: String,
    pub check_id: String,
    pub route: String, // "direct", "proxy"
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub actual_request: String, // "yes", "no", "unknown"
    pub cache_hit: bool,
    pub request_method: String, // "HEAD", "GET"
    pub status_code: Option<u16>,
    pub error_kind: Option<String>,
    pub initial_url: String,
    pub final_url: Option<String>,
    pub redirect_chain: Vec<String>,
    pub redirect_count: usize,
    pub response_time_ms: u64,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub title: Option<String>,
    pub canonical_url: Option<String>,
    pub body_preview: Option<String>,
    pub body_hash: Option<String>,
    pub header_hash: Option<String>,
    pub ip_addresses: Vec<String>,
    pub dns_rcode: Option<String>,
    pub resolver_id: Option<String>,
    pub tls_summary: Option<String>,
    pub screenshot_path: Option<String>,
    pub raw_artifact_path: Option<String>,
    pub confidence: f64,
}

impl Default for BackendObservation {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            backend: "unknown".to_string(),
            backend_version: "0.0.0".to_string(),
            run_id: String::new(),
            check_id: String::new(),
            route: "direct".to_string(),
            started_at: now,
            finished_at: now,
            actual_request: "yes".to_string(),
            cache_hit: false,
            request_method: "GET".to_string(),
            status_code: None,
            error_kind: None,
            initial_url: String::new(),
            final_url: None,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            response_time_ms: 0,
            content_type: None,
            content_length: None,
            title: None,
            canonical_url: None,
            body_preview: None,
            body_hash: None,
            header_hash: None,
            ip_addresses: Vec::new(),
            dns_rcode: None,
            resolver_id: None,
            tls_summary: None,
            screenshot_path: None,
            raw_artifact_path: None,
            confidence: 1.0,
        }
    }
}

/// Context options passed down to probe execution
#[derive(Debug, Clone)]
pub struct ProbeContext {
    pub run_id: String,
    pub check_id: String,
    pub proxy_url: Option<String>,
    pub timeout_secs: u64,
    pub slow_mode: bool,
    pub allow_private_ips: bool,
    pub history_db_path: Option<PathBuf>,
    pub secondary_backends: Vec<String>,
    pub artifact_dir: Option<PathBuf>,
}

/// Information about an external or internal tool backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub tool_id: String,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub available: bool,
    pub supported_features: Vec<String>,
}

/// A canonical, order-stable representation of the engine configuration for hashing.
///
/// All fields that affect *evidence semantics* MUST be included.
/// Fields that affect only display, logging, or output paths MUST NOT be included
/// (e.g. run_id, started_at, output_dir, log_level, random jitter values).
///
/// Uses a Rust struct (field-order stable JSON serialization) — never a raw HashMap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigHashInput {
    /// All enabled backends, canonicalized before serialization.
    pub enabled_backends: Vec<String>,
    /// Direct or proxy routing. Proxy credentials/endpoints are deliberately excluded.
    pub route_strategy: String,
    /// The HTTP method/fallback policy used by the primary probe.
    pub request_strategy: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Whether private IPs are probed.
    pub allow_private_ips: bool,
    /// Current health rule version.
    pub rule_version: String,
    /// Resolver policy used for structured DNS evidence.
    pub dns_resolver_policy: String,
    /// Cache policy used by probes.
    pub cache_policy: String,
    /// Retry behavior that can change observed evidence.
    pub retry_preset: String,
    /// Bump when backend-to-verifier-family mapping semantics change.
    pub verifier_family_mapping_version: String,
}

impl ConfigHashInput {
    pub fn from_probe_context(context: &ProbeContext) -> Self {
        let mut enabled_backends = context.secondary_backends.clone();
        enabled_backends.push("native_reqwest".to_string());
        enabled_backends.sort();
        enabled_backends.dedup();

        Self {
            enabled_backends,
            route_strategy: if context.proxy_url.is_some() {
                "proxy".to_string()
            } else {
                "direct".to_string()
            },
            request_strategy: "head_then_get_html_content_v1".to_string(),
            timeout_secs: context.timeout_secs,
            allow_private_ips: context.allow_private_ips,
            rule_version: crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION.to_string(),
            dns_resolver_policy: "cloudflare_and_alidns".to_string(),
            cache_policy: "no_cache".to_string(),
            retry_preset: "single_attempt".to_string(),
            verifier_family_mapping_version: "1".to_string(),
        }
    }

    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        let mut canonical = self.clone();
        canonical.enabled_backends.sort();
        canonical.enabled_backends.dedup();
        serde_json::to_string(&canonical)
    }

    pub fn stable_hash(&self) -> Result<String, serde_json::Error> {
        self.canonical_json()
            .map(|json| blake3::hash(json.as_bytes()).to_hex().to_string())
    }
}
