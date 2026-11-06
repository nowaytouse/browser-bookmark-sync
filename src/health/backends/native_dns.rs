//! In-process DNS evidence backend.
//!
//! Each instance talks to one independent DNS-over-HTTPS provider.  A resolver
//! failure is deliberately recorded as temporary evidence; it is never
//! treated as proof that a bookmark is dead.

use crate::health::backends::{reject_unsafe_target, ProbeBackend};
use crate::health::models::{BackendObservation, ObservationSource, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use hickory_resolver::config::{
    NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts, CLOUDFLARE,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::proto::rr::{Name, RecordType};
use hickory_resolver::TokioResolver;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const BACKEND_VERSION: &str = "hickory-resolver-0.26.3";
const MAX_CACHE_ENTRIES: usize = 8192;

#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod wire_tests;

#[derive(Clone)]
pub struct NativeDnsBackend {
    resolver: Arc<TokioResolver>,
    resolver_id: &'static str,
    state: Arc<Mutex<ProviderState>>,
}

#[derive(Default)]
struct ProviderState {
    cache: HashMap<String, CachedObservation>,
    last_request: Option<(Instant, Duration)>,
}

struct CachedObservation {
    observation: BackendObservation,
    expires_at: Instant,
}

impl NativeDnsBackend {
    pub fn cloudflare(timeout_secs: u64) -> Result<Self, String> {
        let config = ResolverConfig::https(&CLOUDFLARE);
        Self::build(config, "cloudflare", timeout_secs)
    }

    pub fn alidns(timeout_secs: u64) -> Result<Self, String> {
        let ns1 = NameServerConfig::https(
            "223.5.5.5"
                .parse::<IpAddr>()
                .map_err(|error| format!("invalid AliDNS address: {error}"))?,
            Arc::from("dns.alidns.com"),
            None,
        );
        let ns2 = NameServerConfig::https(
            "223.6.6.6"
                .parse::<IpAddr>()
                .map_err(|error| format!("invalid AliDNS address: {error}"))?,
            Arc::from("dns.alidns.com"),
            None,
        );
        Self::build(
            ResolverConfig::from_parts(None, vec![], vec![ns1, ns2]),
            "alidns",
            timeout_secs,
        )
    }

    fn build(
        config: ResolverConfig,
        resolver_id: &'static str,
        timeout_secs: u64,
    ) -> Result<Self, String> {
        let mut options = ResolverOpts::default();
        options.timeout = Duration::from_secs(timeout_secs.max(1));
        // `attempts` is additional retries, not total sends.
        options.attempts = 0;
        options.num_concurrent_reqs = 1;
        options.use_hosts_file = ResolveHosts::Never;
        // Hickory does not expose per-call cache provenance. Keep one observable
        // cache here instead, so reused negatives cannot look like fresh evidence.
        options.cache_size = 0;
        let resolver = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default())
            .with_options(options)
            .build()
            .map_err(|error| format!("build {resolver_id} resolver: {error}"))?;
        Ok(Self {
            resolver: Arc::new(resolver),
            resolver_id,
            state: Arc::new(Mutex::new(ProviderState::default())),
        })
    }

    fn fill_common(&self, target: &ProbeTarget, context: &ProbeContext) -> BackendObservation {
        let now = Utc::now();
        BackendObservation {
            backend: "hickory_dns".to_string(),
            backend_version: BACKEND_VERSION.to_string(),
            run_id: context.run_id.clone(),
            check_id: context.check_id.clone(),
            route: "direct".to_string(),
            started_at: now,
            finished_at: now,
            actual_request: "no".to_string(),
            request_method: "DNS".to_string(),
            initial_url: target.original_url.clone(),
            resolver_id: Some(self.resolver_id.to_string()),
            ..BackendObservation::default()
        }
    }
}

fn classify_error(error: &NetError, original_name: &Name) -> (&'static str, Option<&'static str>) {
    match error {
        // Hickory propagates the final CNAME target's error, including local
        // special-use answers. Neither its identity nor TTL proves the source
        // name absent; the error also lacks a complete chain TTL.
        NetError::Dns(DnsError::NoRecordsFound(records))
            if records.query.name() != original_name
                || records.query.query_type() != RecordType::A =>
        {
            ("TEMPORARY_ERROR", Some("DnsAliasUncertain"))
        }
        NetError::Dns(DnsError::NoRecordsFound(records))
            if records.response_code == ResponseCode::NXDomain =>
        {
            ("NXDOMAIN", Some("DnsNxdomain"))
        }
        NetError::Dns(DnsError::NoRecordsFound(records))
            if records.response_code == ResponseCode::NoError =>
        {
            ("NOERROR", None)
        }
        // A response code without its query cannot establish source identity.
        _ => ("TEMPORARY_ERROR", Some("DnsTemporaryError")),
    }
}

#[async_trait]
impl ProbeBackend for NativeDnsBackend {
    fn backend_id(&self) -> &'static str {
        "hickory_dns"
    }

    async fn probe(
        &self,
        target: &ProbeTarget,
        context: &ProbeContext,
    ) -> Result<BackendObservation, String> {
        let started = Instant::now();
        let mut observation = self.fill_common(target, context);

        if reject_unsafe_target(target, context, &mut observation) {
            observation.dns_rcode = None;
            observation.response_time_ms = started.elapsed().as_millis() as u64;
            return Ok(observation);
        }

        if context.proxy_url.is_some() {
            observation.error_kind = Some("DnsDirectProbeDisabledForProxy".into());
            return Ok(observation);
        }
        let parsed = reqwest::Url::parse(&target.original_url)
            .map_err(|error| format!("DNS target URL: {error}"))?;
        let host = parsed
            .host_str()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok()
            || ["localhost", "invalid", "local", "onion"]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        {
            // Literal/special-use answers can be synthesized without provider IO.
            // Do not turn them into remote NXDOMAIN or reachability evidence.
            observation.error_kind = Some("DnsNotApplicable".into());
            return Ok(observation);
        }
        let mut name = match Name::from_ascii(&host) {
            Ok(name) => name,
            Err(_) => {
                observation.error_kind = Some("DnsInvalidName".into());
                return Ok(observation);
            }
        };
        name.set_fqdn(true);
        // Serialize cold lookups and cache publication across clones; concurrent
        // callers for the same name must not multiply provider requests.
        let mut state = self.state.lock().await;
        if let Some(cached) = state
            .cache
            .get(&host)
            .filter(|entry| entry.expires_at > Instant::now())
        {
            let mut reused = cached.observation.clone();
            reused.run_id = context.run_id.clone();
            reused.check_id = context.check_id.clone();
            reused.initial_url = target.original_url.clone();
            reused.actual_request = "no".into();
            reused.cache_hit = true;
            return Ok(reused);
        }
        // Bind fresh evidence before any lookup; cached clones retain this
        // original source even when their current batch membership changes.
        observation.source = Some(ObservationSource::from_context(context)?);
        state.cache.remove(&host);
        let interval = Duration::from_millis(if context.slow_mode { 2000 } else { 200 });
        if let Some((last, previous_interval)) = state.last_request {
            let remaining = interval
                .max(previous_interval)
                .saturating_sub(last.elapsed());
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
        }
        let started = Instant::now();
        // Set before awaiting IO, so cancellation cannot bypass the next interval.
        state.last_request = Some((started, interval));
        observation.started_at = Utc::now();
        observation.actual_request = "yes".into();
        // DNS existence does not require parallel A/AAAA address collection. A
        // NOERROR/NODATA response remains non-destructive for IPv6-only names.
        let expires_at = match tokio::time::timeout(
            Duration::from_secs(context.timeout_secs.max(1)),
            self.resolver.lookup(name.clone(), RecordType::A),
        )
        .await
        {
            Ok(Ok(lookup)) => {
                observation.dns_rcode = Some("NOERROR".to_string());
                observation.ip_addresses = lookup
                    .answers()
                    .iter()
                    .filter_map(|record| record.data.ip_addr())
                    .map(|ip| ip.to_string())
                    .collect();
                Some(lookup.valid_until())
            }
            Ok(Err(error)) => {
                let (rcode, error_kind) = classify_error(&error, &name);
                observation.dns_rcode = Some(rcode.to_string());
                observation.error_kind = error_kind.map(str::to_string);
                match &error {
                    NetError::Dns(DnsError::NoRecordsFound(records))
                        if matches!(rcode, "NXDOMAIN" | "NOERROR") =>
                    {
                        records.negative_ttl.and_then(|ttl| {
                            Instant::now().checked_add(Duration::from_secs(u64::from(ttl)))
                        })
                    }
                    _ => None,
                }
            }
            Err(_) => {
                observation.dns_rcode = Some("TEMPORARY_ERROR".into());
                observation.error_kind = Some("DnsTemporaryError".into());
                None
            }
        };
        observation.response_time_ms = started.elapsed().as_millis() as u64;
        observation.finished_at = Utc::now();
        state.last_request = Some((Instant::now(), interval));
        if let Some(expires_at) = expires_at.filter(|at| *at > Instant::now()) {
            if state.cache.len() >= MAX_CACHE_ENTRIES {
                if let Some(key) = state
                    .cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.expires_at)
                    .map(|(key, _)| key.clone())
                {
                    state.cache.remove(&key);
                }
            }
            state.cache.insert(
                host,
                CachedObservation {
                    observation: observation.clone(),
                    expires_at,
                },
            );
        }
        Ok(observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_ids_are_independent() {
        let cloudflare = NativeDnsBackend::cloudflare(1).unwrap();
        let alidns = NativeDnsBackend::alidns(1).unwrap();
        assert_eq!(cloudflare.resolver_id, "cloudflare");
        assert_eq!(alidns.resolver_id, "alidns");
        assert_eq!(cloudflare.backend_id(), "hickory_dns");
        assert_eq!(
            crate::health::state_machine::StateMachine::verifier_family(cloudflare.backend_id()),
            "dns"
        );
    }

    #[test]
    fn only_nxdomain_is_a_strong_dns_failure() {
        let query = hickory_resolver::proto::op::Query::query(
            "missing.example.".parse().unwrap(),
            RecordType::A,
        );
        let nxdomain = NetError::Dns(DnsError::NoRecordsFound(
            hickory_resolver::net::NoRecords::new(query, ResponseCode::NXDomain),
        ));
        let noerror = NetError::Dns(DnsError::NoRecordsFound(
            hickory_resolver::net::NoRecords::new(
                hickory_resolver::proto::op::Query::query(
                    "missing.example.".parse().unwrap(),
                    RecordType::A,
                ),
                ResponseCode::NoError,
            ),
        ));
        let source = "missing.example.".parse().unwrap();
        assert_eq!(classify_error(&nxdomain, &source).0, "NXDOMAIN");
        assert_eq!(classify_error(&noerror, &source).0, "NOERROR");
        assert_eq!(
            classify_error(&NetError::Timeout, &source).0,
            "TEMPORARY_ERROR"
        );
    }
}
