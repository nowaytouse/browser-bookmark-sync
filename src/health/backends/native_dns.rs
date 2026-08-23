//! In-process DNS evidence backend.
//!
//! Each instance talks to one independent DNS-over-HTTPS provider.  A resolver
//! failure is deliberately recorded as temporary evidence; it is never
//! treated as proof that a bookmark is dead.

use crate::health::backends::{reject_unsafe_target, ProbeBackend};
use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;
use chrono::Utc;
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts, CLOUDFLARE};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::TokioResolver;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

const BACKEND_VERSION: &str = "hickory-resolver-0.26.1";

#[derive(Clone)]
pub struct NativeDnsBackend {
    resolver: Arc<TokioResolver>,
    resolver_id: &'static str,
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
        options.attempts = 1;
        let resolver = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default())
            .with_options(options)
            .build()
            .map_err(|error| format!("build {resolver_id} resolver: {error}"))?;
        Ok(Self {
            resolver: Arc::new(resolver),
            resolver_id,
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
            actual_request: "yes".to_string(),
            request_method: "DNS".to_string(),
            initial_url: target.original_url.clone(),
            resolver_id: Some(self.resolver_id.to_string()),
            ..BackendObservation::default()
        }
    }
}

fn classify_error(error: &NetError) -> &'static str {
    match error {
        NetError::Dns(DnsError::NoRecordsFound(records))
            if records.response_code == ResponseCode::NXDomain =>
        {
            "NXDOMAIN"
        }
        NetError::Dns(DnsError::ResponseCode(ResponseCode::NXDomain)) => "NXDOMAIN",
        NetError::Dns(DnsError::NoRecordsFound(records))
            if records.response_code == ResponseCode::NoError =>
        {
            "NOERROR"
        }
        _ => "TEMPORARY_ERROR",
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
        let started = std::time::Instant::now();
        let mut observation = self.fill_common(target, context);

        if reject_unsafe_target(target, context, &mut observation) {
            observation.dns_rcode = None;
            observation.response_time_ms = started.elapsed().as_millis() as u64;
            return Ok(observation);
        }

        match self.resolver.lookup_ip(target.host.as_str()).await {
            Ok(lookup) => {
                observation.dns_rcode = Some("NOERROR".to_string());
                observation.ip_addresses = lookup.iter().map(|ip| ip.to_string()).collect();
            }
            Err(error) => {
                let rcode = classify_error(&error);
                observation.dns_rcode = Some(rcode.to_string());
                observation.error_kind = match rcode {
                    "NXDOMAIN" => Some("DnsNxdomain".to_string()),
                    "NOERROR" => None,
                    _ => Some("DnsTemporaryError".to_string()),
                };
            }
        }
        observation.response_time_ms = started.elapsed().as_millis() as u64;
        observation.finished_at = Utc::now();
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
            hickory_resolver::proto::rr::RecordType::A,
        );
        let nxdomain = NetError::Dns(DnsError::NoRecordsFound(
            hickory_resolver::net::NoRecords::new(query, ResponseCode::NXDomain),
        ));
        let noerror = NetError::Dns(DnsError::NoRecordsFound(
            hickory_resolver::net::NoRecords::new(
                hickory_resolver::proto::op::Query::default(),
                ResponseCode::NoError,
            ),
        ));
        assert_eq!(classify_error(&nxdomain), "NXDOMAIN");
        assert_eq!(classify_error(&noerror), "NOERROR");
        assert_eq!(classify_error(&NetError::Timeout), "TEMPORARY_ERROR");
    }
}
