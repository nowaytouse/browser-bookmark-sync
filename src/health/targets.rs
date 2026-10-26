use crate::health::models::ProbeTarget;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeScope {
    Site,
    Content,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteTarget {
    pub original_url: String,
    pub scheme: String,
    pub normalized_host: String,
    pub effective_port: u16,
    pub registrable_domain: String,
    pub site_key: String,
    pub probe_url: String,
    pub bookmark_count: usize,
    pub source_bookmark_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidTarget {
    pub original_url: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedTarget {
    pub original_url: String,
    pub reason: String,
    #[serde(default)]
    pub site_key: Option<String>,
    #[serde(default)]
    pub bookmark_count: usize,
    #[serde(default)]
    pub source_bookmark_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbePlan {
    pub scope: ProbeScope,
    pub targets: Vec<SiteTarget>,
    pub invalid: Vec<InvalidTarget>,
    pub skipped: Vec<SkippedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedSiteSkipList {
    pub version: String,
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsPolicy {
    #[serde(default = "default_resolvers")]
    pub default_resolvers: Vec<String>,
    #[serde(default = "china_resolvers")]
    pub china_resolvers: Vec<String>,
    #[serde(default = "china_suffixes")]
    pub china_suffixes: Vec<String>,
    #[serde(default)]
    pub domain_overrides: BTreeMap<String, Vec<String>>,
    #[serde(default = "confirmation_resolvers")]
    pub confirmation_resolvers: Vec<String>,
    /// Try a distinct provider on temporary DNS failures, not on security blocks.
    #[serde(default)]
    pub dns_fallback_on_error: bool,
}

impl Default for DnsPolicy {
    fn default() -> Self {
        Self {
            default_resolvers: default_resolvers(),
            china_resolvers: china_resolvers(),
            china_suffixes: china_suffixes(),
            domain_overrides: BTreeMap::new(),
            confirmation_resolvers: confirmation_resolvers(),
            dns_fallback_on_error: false,
        }
    }
}

impl DnsPolicy {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let bytes =
            std::fs::read(path.as_ref()).map_err(|error| format!("read DNS policy: {error}"))?;
        let policy: Self =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse DNS policy: {error}"))?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), String> {
        let known = |resolver: &str| matches!(resolver, "cloudflare" | "alidns");
        let unknown = self
            .default_resolvers
            .iter()
            .chain(&self.china_resolvers)
            .chain(self.domain_overrides.values().flatten())
            .chain(&self.confirmation_resolvers)
            .find(|resolver| !known(resolver));
        if let Some(resolver) = unknown {
            return Err(format!(
                "unsupported DNS resolver id '{resolver}'; supported ids: cloudflare, alidns"
            ));
        }
        if self.default_resolvers.is_empty()
            || self.china_resolvers.is_empty()
            || self.domain_overrides.values().any(Vec::is_empty)
            || self
                .confirmation_resolvers
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                < 2
        {
            return Err(
                "DNS policy requires non-empty routing lists and two confirmation resolvers"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub fn resolver_ids_for_host(&self, host: &str) -> Vec<String> {
        let normalized = host.trim_end_matches('.').to_ascii_lowercase();
        let domain = registrable_domain(&normalized).unwrap_or(normalized.clone());
        if let Some(resolvers) = self
            .domain_overrides
            .iter()
            .find(|(key, _)| normalize_domain_key(key) == domain)
            .map(|(_, value)| value)
        {
            return resolvers.clone();
        }
        if self
            .china_suffixes
            .iter()
            .map(|suffix| suffix.trim().trim_start_matches('.').to_ascii_lowercase())
            .any(|suffix| normalized == suffix || normalized.ends_with(&format!(".{suffix}")))
        {
            self.china_resolvers.clone()
        } else {
            self.default_resolvers.clone()
        }
    }

    pub fn hash(&self) -> Result<String, String> {
        let bytes =
            serde_json::to_vec(self).map_err(|error| format!("serialize DNS policy: {error}"))?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}

fn default_resolvers() -> Vec<String> {
    vec!["cloudflare".to_string()]
}

fn china_resolvers() -> Vec<String> {
    vec!["alidns".to_string()]
}

fn china_suffixes() -> Vec<String> {
    vec![
        ".cn".to_string(),
        ".中国".to_string(),
        ".公司".to_string(),
        ".网络".to_string(),
    ]
}

fn confirmation_resolvers() -> Vec<String> {
    vec!["cloudflare".to_string(), "alidns".to_string()]
}

fn normalize_domain_key(domain: &str) -> String {
    let host = domain
        .trim()
        .trim_end_matches('.')
        .trim_start_matches('.')
        .to_ascii_lowercase();
    registrable_domain(&host).unwrap_or(host)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkTargetInput {
    pub bookmark_id: String,
    pub original_url: String,
}

impl From<&ProbeTarget> for BookmarkTargetInput {
    fn from(target: &ProbeTarget) -> Self {
        Self {
            bookmark_id: target.bookmark_id.clone(),
            original_url: target.original_url.clone(),
        }
    }
}

impl TrustedSiteSkipList {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let bytes =
            std::fs::read(path.as_ref()).map_err(|error| format!("read trusted list: {error}"))?;
        serde_json::from_slice(&bytes).map_err(|error| format!("parse trusted list: {error}"))
    }

    pub fn hash(&self) -> String {
        let mut domains = self.normalized_domains();
        domains.sort();
        blake3::hash(format!("{}\n{}", self.version, domains.join("\n")).as_bytes())
            .to_hex()
            .to_string()
    }

    pub fn matches_host(&self, host: &str) -> Result<bool, String> {
        let Some(registrable) = registrable_domain(host) else {
            return Ok(false);
        };
        Ok(self
            .normalized_domains()
            .iter()
            .any(|domain| domain == &registrable))
    }

    /// `no_trusted_site_skip` is the programmatic equivalent of the CLI `--no-*` switch.
    pub fn should_skip(&self, target: &SiteTarget, no_trusted_site_skip: bool) -> bool {
        !no_trusted_site_skip && self.matches_host(&target.normalized_host).unwrap_or(false)
    }

    fn normalized_domains(&self) -> Vec<String> {
        self.domains
            .iter()
            .filter_map(|domain| {
                let host = domain
                    .trim()
                    .trim_end_matches('.')
                    .trim_start_matches('.')
                    .to_ascii_lowercase();
                registrable_domain(&host).or_else(|| (!host.is_empty()).then_some(host))
            })
            .collect()
    }
}

impl ProbePlan {
    pub fn apply_trusted_site_skip(
        &mut self,
        list: &TrustedSiteSkipList,
        no_trusted_site_skip: bool,
    ) {
        if no_trusted_site_skip {
            return;
        }
        let mut retained = Vec::with_capacity(self.targets.len());
        for target in self.targets.drain(..) {
            if list.should_skip(&target, false) {
                self.skipped.push(SkippedTarget {
                    original_url: target.probe_url.clone(),
                    reason: "SkippedTrustedSite".to_string(),
                    site_key: Some(target.site_key),
                    bookmark_count: target.bookmark_count,
                    source_bookmark_ids: target.source_bookmark_ids,
                });
            } else {
                retained.push(target);
            }
        }
        self.targets = retained;
    }
}

pub fn build_probe_plan(inputs: &[BookmarkTargetInput], scope: ProbeScope) -> ProbePlan {
    let mut grouped = BTreeMap::<String, SiteTarget>::new();
    let mut plan = ProbePlan {
        scope,
        targets: Vec::new(),
        invalid: Vec::new(),
        skipped: Vec::new(),
    };

    for input in inputs {
        let parsed = match reqwest::Url::parse(&input.original_url) {
            Ok(parsed) => parsed,
            Err(error) => {
                plan.invalid.push(InvalidTarget {
                    original_url: input.original_url.clone(),
                    reason: format!("InvalidUrl: {error}"),
                });
                continue;
            }
        };
        let scheme = parsed.scheme().to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https") {
            plan.skipped.push(SkippedTarget {
                original_url: input.original_url.clone(),
                reason: "UnsupportedScheme".to_string(),
                site_key: None,
                bookmark_count: 1,
                source_bookmark_ids: vec![input.bookmark_id.clone()],
            });
            continue;
        }
        let Some(host) = parsed.host_str() else {
            plan.invalid.push(InvalidTarget {
                original_url: input.original_url.clone(),
                reason: "MissingHost".to_string(),
            });
            continue;
        };
        let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
        if normalized_host.is_empty() {
            plan.invalid.push(InvalidTarget {
                original_url: input.original_url.clone(),
                reason: "MissingHost".to_string(),
            });
            continue;
        }
        if normalized_host == "onion" || normalized_host.ends_with(".onion") {
            plan.skipped.push(SkippedTarget {
                original_url: input.original_url.clone(),
                reason: "TorHiddenService".to_string(),
                site_key: None,
                bookmark_count: 1,
                source_bookmark_ids: vec![input.bookmark_id.clone()],
            });
            continue;
        }
        let effective_port = parsed
            .port_or_known_default()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let Some(registrable) = registrable_domain(&normalized_host) else {
            plan.invalid.push(InvalidTarget {
                original_url: input.original_url.clone(),
                reason: "NoRegistrableDomain".to_string(),
            });
            continue;
        };
        let (site_key, probe_url) = match scope {
            ProbeScope::Site => {
                let key = format!("{scheme}://{registrable}:{effective_port}");
                let url_host = format_url_host(&registrable);
                let port = known_default_port(&scheme, effective_port)
                    .map_or_else(|| format!(":{effective_port}"), |_| String::new());
                (key, format!("{scheme}://{url_host}{port}/"))
            }
            ProbeScope::Content => (
                format!("content:{}", input.original_url),
                input.original_url.clone(),
            ),
        };
        let target = grouped
            .entry(site_key.clone())
            .or_insert_with(|| SiteTarget {
                original_url: input.original_url.clone(),
                scheme: scheme.clone(),
                normalized_host: normalized_host.clone(),
                effective_port,
                registrable_domain: registrable.clone(),
                site_key,
                probe_url,
                bookmark_count: 0,
                source_bookmark_ids: Vec::new(),
            });
        target.bookmark_count += 1;
        target.source_bookmark_ids.push(input.bookmark_id.clone());
    }
    plan.targets = grouped.into_values().collect();
    plan
}

pub fn build_probe_plan_from_probe_targets(
    targets: &[ProbeTarget],
    scope: ProbeScope,
) -> ProbePlan {
    let inputs = targets
        .iter()
        .map(BookmarkTargetInput::from)
        .collect::<Vec<_>>();
    build_probe_plan(&inputs, scope)
}

pub(crate) fn registrable_domain(host: &str) -> Option<String> {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    // URL host_str retains brackets for IPv6 literals. They are syntax, not a
    // public suffix; normalize the IP without treating bracketed names as IPs.
    let literal = normalized
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&normalized);
    if let Ok(address) = literal.parse::<IpAddr>() {
        return Some(address.to_string());
    }
    psl::domain_str(&normalized).map(str::to_ascii_lowercase)
}

fn format_url_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn known_default_port(scheme: &str, port: u16) -> Option<String> {
    match (scheme, port) {
        ("http", 80) | ("https", 443) => Some(String::new()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> ProbeTarget {
        ProbeTarget::from_url(url, url).unwrap()
    }

    #[test]
    fn site_scope_uses_psl_and_groups_hosts() {
        let plan = build_probe_plan(
            &[
                (&target("https://www.example.co.uk/a")).into(),
                (&target("https://blog.example.co.uk/b")).into(),
            ],
            ProbeScope::Site,
        );
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].registrable_domain, "example.co.uk");
        assert_eq!(plan.targets[0].bookmark_count, 2);
        assert_eq!(plan.targets[0].site_key, "https://example.co.uk:443");
        assert_eq!(plan.targets[0].probe_url, "https://example.co.uk/");
    }

    #[test]
    fn public_suffix_cases_are_not_last_two_labels() {
        for (host, expected) in [
            ("repo.github.io", "repo.github.io"),
            ("blog.blogspot.co.uk", "blogspot.co.uk"),
            ("www.example.co.uk", "example.co.uk"),
            ("www.example.com.cn", "example.com.cn"),
        ] {
            let plan = build_probe_plan(
                &[(&target(&format!("https://{host}/path"))).into()],
                ProbeScope::Site,
            );
            assert_eq!(plan.targets[0].registrable_domain, expected, "{host}");
        }
    }

    #[test]
    fn bare_suffixes_and_single_label_hosts_are_invalid_but_ips_are_explicit() {
        let inputs = ["https://co.uk/", "https://com.cn/", "http://localhost/"]
            .into_iter()
            .enumerate()
            .map(|(index, original_url)| BookmarkTargetInput {
                bookmark_id: index.to_string(),
                original_url: original_url.to_string(),
            })
            .collect::<Vec<_>>();
        let plan = build_probe_plan(&inputs, ProbeScope::Site);
        assert!(plan.targets.is_empty());
        assert_eq!(plan.invalid.len(), 3);

        let ip = build_probe_plan(
            &[BookmarkTargetInput {
                bookmark_id: "ip".to_string(),
                original_url: "https://203.0.113.7/path".to_string(),
            }],
            ProbeScope::Site,
        );
        assert_eq!(ip.targets[0].registrable_domain, "203.0.113.7");
    }

    #[test]
    fn content_scope_keeps_complete_urls_and_ports_distinct() {
        let plan = build_probe_plan(
            &[
                (&target("https://example.com/a")).into(),
                (&target("https://example.com/b")).into(),
                (&target("https://example.com:8443/a")).into(),
            ],
            ProbeScope::Content,
        );
        assert_eq!(plan.targets.len(), 3);
        assert_eq!(plan.targets[0].probe_url, "https://example.com/a");
        assert_eq!(plan.targets[2].effective_port, 8443);
    }

    #[test]
    fn trusted_matching_is_exact_registrable_domain_and_disableable() {
        let list = TrustedSiteSkipList {
            version: "1".to_string(),
            domains: vec!["youtube.com".to_string()],
        };
        assert!(list.matches_host("www.youtube.com").unwrap());
        assert!(list.matches_host("m.youtube.com").unwrap());
        assert!(!list.matches_host("notyoutube.com").unwrap());
        let plan = build_probe_plan(
            &[(&target("https://www.youtube.com/watch?v=1")).into()],
            ProbeScope::Site,
        );
        assert!(list.should_skip(&plan.targets[0], false));
        assert!(!list.should_skip(&plan.targets[0], true));
    }

    #[test]
    fn trusted_json_has_version_hash_and_explicit_skip_output() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("trusted_site_skip_list.json");
        let list = TrustedSiteSkipList::load(path).unwrap();
        assert_eq!(list.version, "1");
        assert_eq!(list.hash().len(), 64);

        let mut plan = build_probe_plan(
            &[
                (&target("https://www.youtube.com/watch?v=1")).into(),
                (&target("https://example.com/page")).into(),
            ],
            ProbeScope::Site,
        );
        plan.apply_trusted_site_skip(&list, false);
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.skipped[0].reason, "SkippedTrustedSite");
    }

    #[test]
    fn dns_policy_routes_cn_and_supports_exact_registrable_override() {
        let mut policy = DnsPolicy::default();
        assert_eq!(policy.resolver_ids_for_host("www.example.cn"), ["alidns"]);
        assert_eq!(
            policy.resolver_ids_for_host("www.example.com"),
            ["cloudflare"]
        );
        policy
            .domain_overrides
            .insert("special.co.uk".to_string(), vec!["alidns".to_string()]);
        assert_eq!(
            policy.resolver_ids_for_host("www.special.co.uk"),
            ["alidns"]
        );
        assert_eq!(
            policy.resolver_ids_for_host("notexample.com"),
            ["cloudflare"]
        );
    }

    #[test]
    fn dns_confirmation_requires_distinct_providers_and_old_policy_has_no_error_fallback() {
        let mut policy: DnsPolicy = serde_json::from_str("{}").unwrap();
        assert!(!policy.dns_fallback_on_error);
        policy.confirmation_resolvers = vec!["cloudflare".into(), "cloudflare".into()];
        assert!(policy.validate().is_err());
    }

    #[test]
    fn dns_policy_rejects_unknown_resolver_ids() {
        let mut policy = DnsPolicy::default();
        policy
            .domain_overrides
            .insert("example.com".to_string(), vec!["custom".to_string()]);
        assert!(policy.validate().unwrap_err().contains("custom"));
    }

    #[test]
    fn invalid_and_skipped_inputs_are_explicit() {
        let plan = build_probe_plan(
            &[
                BookmarkTargetInput {
                    bookmark_id: "js".to_string(),
                    original_url: "javascript:void(0)".to_string(),
                },
                BookmarkTargetInput {
                    bookmark_id: "bad".to_string(),
                    original_url: "https://".to_string(),
                },
            ],
            ProbeScope::Site,
        );
        assert_eq!(plan.targets.len(), 0);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.invalid.len(), 1);
    }

    #[test]
    fn onion_targets_are_never_planned_for_network_access() {
        let plan = build_probe_plan(
            &[BookmarkTargetInput {
                bookmark_id: "onion".to_string(),
                original_url: "http://examplehiddenservice.onion/post".to_string(),
            }],
            ProbeScope::Content,
        );
        assert!(plan.targets.is_empty());
        assert_eq!(plan.skipped[0].reason, "TorHiddenService");
    }
}
