//! Tier 0 Security Pre-checks for URL Validation

use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityCheckResult {
    Allowed,
    Denied(String),
}

/// Perform Tier 0 Security Pre-check on parsed Url
pub fn validate_url_parsed(parsed: &Url, allow_private: bool) -> SecurityCheckResult {
    // Scheme check
    let scheme = parsed.scheme().to_lowercase();
    if scheme != "http" && scheme != "https" {
        return SecurityCheckResult::Denied(format!("Unsupported URI scheme: {scheme}"));
    }

    // Host check
    let host_str = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return SecurityCheckResult::Denied("Missing host in URL".to_string()),
    };

    // Onion check
    if host_str.ends_with(".onion") {
        return SecurityCheckResult::Denied("Tor .onion domain blocked by default".to_string());
    }

    // Localhost check
    if (host_str == "localhost" || host_str.ends_with(".localhost") || host_str.ends_with(".local"))
        && !allow_private
    {
        return SecurityCheckResult::Denied("Localhost host blocked by default".to_string());
    }

    // IP address checks
    if let Ok(ip) = host_str.parse::<IpAddr>() {
        if let Err(reason) = is_ip_allowed(ip, allow_private) {
            return SecurityCheckResult::Denied(reason);
        }
    }

    SecurityCheckResult::Allowed
}

/// Perform Tier 0 Security Pre-check on URL target string
pub fn validate_url_security(url_str: &str, allow_private: bool) -> SecurityCheckResult {
    let parsed = match Url::parse(url_str) {
        Ok(u) => u,
        Err(e) => return SecurityCheckResult::Denied(format!("Invalid URL syntax: {e}")),
    };
    validate_url_parsed(&parsed, allow_private)
}

/// Check if a resolved IP address is allowed
pub fn is_ip_allowed(ip: IpAddr, allow_private: bool) -> Result<(), String> {
    if allow_private {
        return Ok(());
    }

    match ip {
        IpAddr::V4(ip4) => {
            // Loopback 127.0.0.0/8
            if ip4.is_loopback() {
                return Err("Loopback IPv4 address blocked".to_string());
            }
            // Private RFC1918
            if ip4.is_private() {
                return Err("RFC1918 Private IPv4 address blocked".to_string());
            }
            // Link local 169.254.0.0/16
            if ip4.is_link_local() {
                return Err("Link-local IPv4 address blocked".to_string());
            }
            // Multicast 224.0.0.0/4
            if ip4.is_multicast() {
                return Err("Multicast IPv4 address blocked".to_string());
            }
            // Metadata service 169.254.169.254
            if ip4 == Ipv4Addr::new(169, 254, 169, 254) {
                return Err("Cloud metadata IP 169.254.169.254 blocked".to_string());
            }
            // Broadcast 255.255.255.255
            if ip4.is_broadcast() {
                return Err("Broadcast IPv4 address blocked".to_string());
            }
            // Unspecified 0.0.0.0
            if ip4.is_unspecified() {
                return Err("Unspecified IPv4 address blocked".to_string());
            }
        }
        IpAddr::V6(ip6) => {
            // Loopback ::1
            if ip6.is_loopback() {
                return Err("Loopback IPv6 address blocked".to_string());
            }
            // Unspecified ::
            if ip6.is_unspecified() {
                return Err("Unspecified IPv6 address blocked".to_string());
            }
            // Multicast ff00::/8
            if ip6.is_multicast() {
                return Err("Multicast IPv6 address blocked".to_string());
            }
            // IPv6 ULA fc00::/7
            let segments = ip6.segments();
            if (segments[0] & 0xfe00) == 0xfc00 {
                return Err("IPv6 ULA address blocked".to_string());
            }
            // Link-local fe80::/10
            if (segments[0] & 0xffc0) == 0xfe80 {
                return Err("IPv6 Link-local address blocked".to_string());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_schemes() {
        assert_eq!(
            validate_url_security("http://example.com", false),
            SecurityCheckResult::Allowed
        );
        assert_eq!(
            validate_url_security("https://example.com", false),
            SecurityCheckResult::Allowed
        );
        assert!(matches!(
            validate_url_security("javascript:alert(1)", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("file:///etc/passwd", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("ftp://ftp.example.com", false),
            SecurityCheckResult::Denied(_)
        ));
    }

    #[test]
    fn test_private_ips() {
        assert!(matches!(
            validate_url_security("http://127.0.0.1", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("http://192.168.1.1", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("http://10.0.0.1", false),
            SecurityCheckResult::Denied(_)
        ));
        assert!(matches!(
            validate_url_security("http://169.254.169.254", false),
            SecurityCheckResult::Denied(_)
        ));
        assert_eq!(
            validate_url_security("http://127.0.0.1", true),
            SecurityCheckResult::Allowed
        );
    }
}
