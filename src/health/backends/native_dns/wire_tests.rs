//! Local DNS replies at the resolver boundary, observed through public probes.

use super::*;
use crate::health::targets::ProbeScope;
use hickory_resolver::config::ConnectionConfig;
use hickory_resolver::proto::op::{Message, ResponseCode};
use hickory_resolver::proto::rr::{
    rdata::{CNAME, SOA},
    Name, RData, Record, RecordType,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct AbortServer(tokio::task::AbortHandle);

impl Drop for AbortServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) fn context(check_id: &str) -> ProbeContext {
    ProbeContext {
        run_id: format!("dns-wire-{check_id}"),
        check_id: check_id.into(),
        proxy_url: None,
        timeout_secs: 1,
        slow_mode: false,
        probe_scope: ProbeScope::Site,
        dns_policy_hash: "synthetic-local-resolver".into(),
        allow_private_ips: false,
        history_db_path: None,
        secondary_backends: Vec::new(),
        artifact_dir: None,
    }
}

#[derive(Clone, Debug)]
struct WireQuery {
    name: Name,
    record_type: RecordType,
    received_at: Instant,
}

pub(super) struct DnsFixture {
    pub(super) backend: NativeDnsBackend,
    pub(super) wire_count: Arc<AtomicUsize>,
    queries: Arc<Mutex<Vec<WireQuery>>>,
    _server_guard: AbortServer,
}

pub(super) async fn fixture(
    rcode: ResponseCode,
    negative_ttl: u32,
) -> Result<DnsFixture, Box<dyn std::error::Error>> {
    fixture_for_provider(rcode, negative_ttl, "fixture-provider").await
}

pub(super) async fn fixture_for_provider(
    rcode: ResponseCode,
    negative_ttl: u32,
    resolver_id: &'static str,
) -> Result<DnsFixture, Box<dyn std::error::Error>> {
    // This socket answers locally; neither query hostname nor provider is sent
    // to a public resolver. Both A and AAAA from the production strategy work.
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
    let address = socket.local_addr()?;
    let wire_count = Arc::new(AtomicUsize::new(0));
    let received = Arc::clone(&wire_count);
    let queries = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&queries);
    let server: tokio::task::JoinHandle<std::io::Result<()>> = tokio::spawn(async move {
        let mut bytes = [0_u8; 4096];
        loop {
            let (length, peer) = socket.recv_from(&mut bytes).await?;
            let query = Message::from_vec(&bytes[..length]).map_err(std::io::Error::other)?;
            received.fetch_add(1, Ordering::SeqCst);
            recorded
                .lock()
                .await
                .extend(query.queries.iter().map(|query| WireQuery {
                    name: query.name().clone(),
                    record_type: query.query_type(),
                    received_at: Instant::now(),
                }));
            let mut reply = Message::error_msg(query.metadata.id, query.metadata.op_code, rcode);
            reply.metadata.authoritative = true;
            reply.metadata.recursion_desired = query.metadata.recursion_desired;
            reply.metadata.recursion_available = true;
            reply.queries = query.queries;
            reply.add_authority(Record::from_rdata(
                Name::from_ascii("example.test.").map_err(std::io::Error::other)?,
                negative_ttl,
                RData::SOA(SOA::new(
                    Name::from_ascii("ns.example.test.").map_err(std::io::Error::other)?,
                    Name::from_ascii("hostmaster.example.test.").map_err(std::io::Error::other)?,
                    1,
                    60,
                    60,
                    60,
                    negative_ttl,
                )),
            ));
            let reply = reply.to_vec().map_err(std::io::Error::other)?;
            socket.send_to(&reply, peer).await?;
        }
    });
    let server_guard = AbortServer(server.abort_handle());
    let mut connection = ConnectionConfig::udp();
    connection.port = address.port();
    let config = ResolverConfig::from_parts(
        None,
        vec![],
        vec![NameServerConfig::new(address.ip(), true, vec![connection])],
    );
    let backend = NativeDnsBackend::build(config, resolver_id, 1)?;
    Ok(DnsFixture {
        backend,
        wire_count,
        queries,
        _server_guard: server_guard,
    })
}

async fn cname_fixture(
    canonical_target: &str,
    target_zone: &str,
    cname_ttl: u32,
    target_negative_ttl: u32,
) -> Result<DnsFixture, Box<dyn std::error::Error>> {
    // The original name exists. Only its CNAME target is absent; these are
    // different DNS identities even if the resolver follows the chain for us.
    let source = Name::from_ascii("alias.example.com.")?;
    let canonical_target = Name::from_ascii(canonical_target)?;
    let target_zone = Name::from_ascii(target_zone)?;
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
    let address = socket.local_addr()?;
    let wire_count = Arc::new(AtomicUsize::new(0));
    let received = Arc::clone(&wire_count);
    let queries = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&queries);
    let server: tokio::task::JoinHandle<std::io::Result<()>> = tokio::spawn(async move {
        let mut bytes = [0_u8; 4096];
        loop {
            let (length, peer) = socket.recv_from(&mut bytes).await?;
            let query = Message::from_vec(&bytes[..length]).map_err(std::io::Error::other)?;
            received.fetch_add(1, Ordering::SeqCst);
            recorded
                .lock()
                .await
                .extend(query.queries.iter().map(|query| WireQuery {
                    name: query.name().clone(),
                    record_type: query.query_type(),
                    received_at: Instant::now(),
                }));
            let is_source = query
                .queries
                .first()
                .is_some_and(|query| query.name() == &source);
            let mut reply = Message::error_msg(
                query.metadata.id,
                query.metadata.op_code,
                if is_source {
                    ResponseCode::NoError
                } else {
                    ResponseCode::NXDomain
                },
            );
            reply.metadata.authoritative = true;
            reply.metadata.recursion_desired = query.metadata.recursion_desired;
            reply.metadata.recursion_available = true;
            reply.queries = query.queries;
            if is_source {
                reply.add_answer(Record::from_rdata(
                    source.clone(),
                    cname_ttl,
                    RData::CNAME(CNAME(canonical_target.clone())),
                ));
            } else {
                reply.add_authority(Record::from_rdata(
                    target_zone.clone(),
                    target_negative_ttl,
                    RData::SOA(SOA::new(
                        Name::from_ascii("ns.example.com.").map_err(std::io::Error::other)?,
                        Name::from_ascii("hostmaster.example.com.")
                            .map_err(std::io::Error::other)?,
                        1,
                        60,
                        60,
                        60,
                        target_negative_ttl,
                    )),
                ));
            }
            let reply = reply.to_vec().map_err(std::io::Error::other)?;
            socket.send_to(&reply, peer).await?;
        }
    });
    let server_guard = AbortServer(server.abort_handle());
    let mut connection = ConnectionConfig::udp();
    connection.port = address.port();
    let config = ResolverConfig::from_parts(
        None,
        vec![],
        vec![NameServerConfig::new(address.ip(), true, vec![connection])],
    );
    let backend = NativeDnsBackend::build(config, "fixture-cname-provider", 1)?;
    Ok(DnsFixture {
        backend,
        wire_count,
        queries,
        _server_guard: server_guard,
    })
}

#[tokio::test]
async fn special_use_cname_target_does_not_fabricate_source_nxdomain(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = cname_fixture("missing.invalid.", "invalid.", 60, 60).await?;
    let target = ProbeTarget::from_url("alias", "http://alias.example.com/article")?;
    let observation = tokio::time::timeout(
        Duration::from_secs(3),
        fixture
            .backend
            .probe(&target, &context("special-use-alias")),
    )
    .await??;
    assert_eq!(observation.actual_request, "yes");
    assert!(!observation.cache_hit);
    assert!(fixture.queries.lock().await.iter().any(|query| {
        query.name.to_ascii() == "alias.example.com." && query.record_type == RecordType::A
    }));
    assert_ne!(
        observation.dns_rcode.as_deref(),
        Some("NXDOMAIN"),
        "a locally synthesized NXDOMAIN for a CNAME target is not source-domain evidence"
    );
    assert_ne!(observation.error_kind.as_deref(), Some("DnsNxdomain"));
    let verdict = crate::health::state_machine::StateMachine::compute_single_run_verdict_for_scope(
        &[observation],
        ProbeScope::Site,
    );
    assert!(!matches!(
        verdict,
        crate::health::models::HealthVerdict::ProbableDead
            | crate::health::models::HealthVerdict::ConfirmedDead
    ));
    Ok(())
}

#[tokio::test]
async fn cname_target_negative_ttl_does_not_outlive_the_source_chain(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = cname_fixture("gone.example.com.", "example.com.", 1, 60).await?;
    let target = ProbeTarget::from_url("alias-ttl", "http://alias.example.com/article")?;
    let first = tokio::time::timeout(
        Duration::from_secs(3),
        fixture
            .backend
            .probe(&target, &context("alias-before-expiry")),
    )
    .await??;
    assert_eq!(first.actual_request, "yes");
    assert!(!first.cache_hit);
    let count_after_first = fixture.wire_count.load(Ordering::SeqCst);
    assert!(count_after_first > 0);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let second = tokio::time::timeout(
        Duration::from_secs(3),
        fixture
            .backend
            .probe(&target, &context("alias-after-expiry")),
    )
    .await??;
    assert!(
        !second.cache_hit,
        "the target's 60-second negative TTL cannot extend the source's 1-second CNAME TTL"
    );
    assert_eq!(second.actual_request, "yes");
    assert!(second.started_at > first.started_at);
    assert!(
        fixture.wire_count.load(Ordering::SeqCst) > count_after_first,
        "the source chain must be checked again after its own TTL expires"
    );
    for observation in [first, second] {
        assert_ne!(
            observation.dns_rcode.as_deref(),
            Some("NXDOMAIN"),
            "a missing canonical target does not prove that the original alias name is absent"
        );
        assert_ne!(observation.error_kind.as_deref(), Some("DnsNxdomain"));
        let verdict =
            crate::health::state_machine::StateMachine::compute_single_run_verdict_for_scope(
                &[observation],
                ProbeScope::Site,
            );
        assert!(!matches!(
            verdict,
            crate::health::models::HealthVerdict::ProbableDead
                | crate::health::models::HealthVerdict::ConfirmedDead
        ));
    }
    Ok(())
}

#[tokio::test]
async fn repeated_cached_nxdomain_is_not_a_new_network_observation(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::NXDomain, 60).await?;
    let backend = &fixture.backend;
    let wire_count = &fixture.wire_count;
    let target = ProbeTarget::from_url("cache", "http://cache.example.test/article")?;
    let first = tokio::time::timeout(
        Duration::from_secs(3),
        backend.probe(&target, &context("first")),
    )
    .await??;
    assert_eq!(first.dns_rcode.as_deref(), Some("NXDOMAIN"));
    assert_eq!(first.actual_request, "yes");
    assert!(!first.cache_hit);
    let count_after_first = wire_count.load(Ordering::SeqCst);
    assert!(
        count_after_first > 0,
        "the initial DNS probe never reached the fixture"
    );

    let second = tokio::time::timeout(
        Duration::from_secs(3),
        backend.probe(&target, &context("second")),
    )
    .await??;
    assert_eq!(second.dns_rcode.as_deref(), Some("NXDOMAIN"));
    assert_eq!(
        wire_count.load(Ordering::SeqCst),
        count_after_first,
        "a cached negative result must not add provider traffic"
    );
    assert_eq!(
        second.actual_request, "no",
        "a cached NXDOMAIN must not masquerade as new network evidence"
    );
    assert!(
        second.cache_hit,
        "the reused negative result must disclose its cache provenance"
    );
    assert_eq!(
        second.started_at, first.started_at,
        "cache reuse must retain the original observation time"
    );
    assert_eq!(first.run_id, "dns-wire-first");
    assert_eq!(first.check_id, "first");
    assert_eq!(second.run_id, "dns-wire-second");
    assert_eq!(second.check_id, "second");
    Ok(())
}

#[tokio::test]
async fn one_a_query_per_uncached_probe_without_parallel_family_amplification(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::NXDomain, 0).await?;
    let target = ProbeTarget::from_url("once", "http://once.example.test/article")?;
    let began = Instant::now();
    for (index, check) in ["first", "second"].into_iter().enumerate() {
        let observation = tokio::time::timeout(
            Duration::from_secs(3),
            fixture.backend.probe(&target, &context(check)),
        )
        .await??;
        assert_eq!(observation.dns_rcode.as_deref(), Some("NXDOMAIN"));
        assert_eq!(observation.actual_request, "yes");
        assert!(
            !observation.cache_hit,
            "zero TTL cannot produce a cache hit"
        );
        assert_eq!(
            fixture.wire_count.load(Ordering::SeqCst),
            index + 1,
            "one uncached existence probe must not fan out across A and AAAA"
        );
    }
    let queries = fixture.queries.lock().await;
    assert_eq!(queries.len(), 2);
    for query in queries.iter() {
        assert_eq!(query.record_type, RecordType::A);
        assert_eq!(query.name.to_ascii(), "once.example.test.");
        assert!(query.received_at >= began);
    }
    Ok(())
}

#[tokio::test]
async fn two_names_share_one_provider_pacing() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::NXDomain, 0).await?;
    let first_backend = fixture.backend.clone();
    let second_backend = fixture.backend.clone();
    let first_target = ProbeTarget::from_url("one", "http://one.example.test/article")?;
    let second_target = ProbeTarget::from_url("two", "http://two.example.test/article")?;
    let mut first_context = context("provider-one");
    first_context.slow_mode = true;
    let mut second_context = context("provider-two");
    second_context.slow_mode = true;
    let (first, second) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            first_backend.probe(&first_target, &first_context),
            second_backend.probe(&second_target, &second_context),
        )
    })
    .await?;
    let first = first?;
    let second = second?;
    assert_eq!(first.check_id, "provider-one");
    assert_eq!(second.check_id, "provider-two");
    for observation in [&first, &second] {
        assert_eq!(observation.dns_rcode.as_deref(), Some("NXDOMAIN"));
        assert_eq!(observation.actual_request, "yes");
        assert!(!observation.cache_hit);
    }
    assert_eq!(fixture.wire_count.load(Ordering::SeqCst), 2);
    let queries = fixture.queries.lock().await;
    assert_eq!(queries.len(), 2);
    let names: std::collections::BTreeSet<_> =
        queries.iter().map(|query| query.name.to_ascii()).collect();
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "one.example.test.".to_string(),
            "two.example.test.".to_string(),
        ])
    );
    assert!(queries
        .iter()
        .all(|query| query.record_type == RecordType::A));
    assert!(
        queries[1]
            .received_at
            .duration_since(queries[0].received_at)
            >= Duration::from_millis(1950),
        "different names bypassed their shared provider's slow-mode interval: {queries:?}"
    );
    Ok(())
}

#[tokio::test]
async fn literal_special_use_and_proxy_targets_do_not_contact_dns_provider(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::NXDomain, 60).await?;
    for (index, url) in [
        "http://93.184.216.34/article",
        "http://[2606:4700:4700::1111]/article",
        "http://missing.invalid/article",
        "http://printer.local/article",
    ]
    .into_iter()
    .enumerate()
    {
        let target = ProbeTarget::from_url(format!("skip-{index}"), url)?;
        let observation = tokio::time::timeout(
            Duration::from_secs(1),
            fixture.backend.probe(&target, &context("skip")),
        )
        .await??;
        assert_eq!(
            observation.actual_request, "no",
            "unexpected DNS request for {url}"
        );
        assert_ne!(
            observation.dns_rcode.as_deref(),
            Some("NXDOMAIN"),
            "a local skip must not fabricate NXDOMAIN for {url}"
        );
        assert_eq!(fixture.wire_count.load(Ordering::SeqCst), 0);
    }
    let mut proxy_context = context("proxy-skip");
    proxy_context.proxy_url = Some("http://127.0.0.1:9".into());
    let target = ProbeTarget::from_url("proxy", "http://proxy.example.test/article")?;
    let observation = tokio::time::timeout(
        Duration::from_secs(1),
        fixture.backend.probe(&target, &proxy_context),
    )
    .await??;
    assert_eq!(observation.actual_request, "no");
    assert_eq!(observation.dns_rcode, None);
    assert_eq!(fixture.wire_count.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn empty_noerror_a_response_does_not_mark_an_ipv6_only_name_dead(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::NoError, 60).await?;
    let target = ProbeTarget::from_url("nodata", "http://ipv6-only.example.test/article")?;
    let observation = tokio::time::timeout(
        Duration::from_secs(3),
        fixture.backend.probe(&target, &context("nodata")),
    )
    .await??;
    assert_eq!(fixture.wire_count.load(Ordering::SeqCst), 1);
    assert_eq!(observation.actual_request, "yes");
    assert!(!observation.cache_hit);
    assert_eq!(observation.dns_rcode.as_deref(), Some("NOERROR"));
    assert!(observation.ip_addresses.is_empty());
    assert_eq!(observation.error_kind, None);
    let verdict = crate::health::state_machine::StateMachine::compute_single_run_verdict_for_scope(
        &[observation],
        ProbeScope::Site,
    );
    assert!(!matches!(
        verdict,
        crate::health::models::HealthVerdict::ProbableDead
            | crate::health::models::HealthVerdict::ConfirmedDead
    ));
    Ok(())
}

#[tokio::test]
async fn expired_negative_ttl_requires_new_network_evidence(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::NXDomain, 1).await?;
    let target = ProbeTarget::from_url("expiry", "http://expiry.example.test/article")?;
    let first = tokio::time::timeout(
        Duration::from_secs(3),
        fixture.backend.probe(&target, &context("before-expiry")),
    )
    .await??;
    assert_eq!(fixture.wire_count.load(Ordering::SeqCst), 1);
    assert_eq!(first.dns_rcode.as_deref(), Some("NXDOMAIN"));
    assert_eq!(first.actual_request, "yes");
    assert!(!first.cache_hit);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let second = tokio::time::timeout(
        Duration::from_secs(3),
        fixture.backend.probe(&target, &context("after-expiry")),
    )
    .await??;
    assert_eq!(fixture.wire_count.load(Ordering::SeqCst), 2);
    assert_eq!(second.dns_rcode.as_deref(), Some("NXDOMAIN"));
    assert_eq!(second.actual_request, "yes");
    assert!(
        !second.cache_hit,
        "expired negative results cannot be reused"
    );
    assert!(second.started_at > first.started_at);
    assert_eq!(second.check_id, "after-expiry");
    Ok(())
}

#[tokio::test]
async fn temporary_dns_failure_is_not_cached_or_retried_inside_one_probe(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(ResponseCode::ServFail, 60).await?;
    let target = ProbeTarget::from_url("temporary", "http://temporary.example.test/article")?;
    for (index, check) in ["temporary-first", "temporary-second"]
        .into_iter()
        .enumerate()
    {
        let observation = tokio::time::timeout(
            Duration::from_secs(3),
            fixture.backend.probe(&target, &context(check)),
        )
        .await??;
        assert_eq!(observation.dns_rcode.as_deref(), Some("TEMPORARY_ERROR"));
        assert_eq!(observation.error_kind.as_deref(), Some("DnsTemporaryError"));
        assert_eq!(observation.actual_request, "yes");
        assert!(
            !observation.cache_hit,
            "transient failures must not become cached proof"
        );
        assert_eq!(
            fixture.wire_count.load(Ordering::SeqCst),
            index + 1,
            "one logical probe must not silently retry its failed DNS request"
        );
    }
    Ok(())
}
