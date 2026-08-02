//! HTML Content Analysis and Soft-404 Detection

use scraper::{Html, Selector};

/// Extract <title> from HTML string
pub fn extract_html_title(html_str: &str) -> Option<String> {
    let fragment = Html::parse_document(html_str);
    let selector = Selector::parse("title").ok()?;
    if let Some(element) = fragment.select(&selector).next() {
        let text = element.text().collect::<Vec<_>>().join(" ");
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Extract <link rel="canonical" href="..."> from HTML string
pub fn extract_canonical_url(html_str: &str) -> Option<String> {
    let fragment = Html::parse_document(html_str);
    let selector = Selector::parse("link[rel='canonical']").ok()?;
    if let Some(element) = fragment.select(&selector).next() {
        if let Some(href) = element.value().attr("href") {
            let trimmed = href.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Detect potential Soft 404 (e.g. 200 OK returning "Page Not Found", "404", "Domain Expired")
pub fn is_soft_404(title: Option<&str>, body_preview: Option<&str>) -> bool {
    let title_lower = title.unwrap_or("").to_lowercase();
    let body_lower = body_preview.unwrap_or("").to_lowercase();

    let keywords = [
        "page not found",
        "404 not found",
        "404 error",
        "domain for sale",
        "domain expired",
        "this domain is for sale",
        "buy this domain",
        "page does not exist",
        "article not found",
        "404 页面不存在",
        "页面找不到",
        "网站防火墙",
    ];

    for kw in &keywords {
        if title_lower.contains(kw) || body_lower.contains(kw) {
            return true;
        }
    }

    false
}

/// Check if a URL belongs to an exempt special domain (.onion, darknet, internal networks)
/// that cannot be accessed normally and should default to Protected (exemption).
pub fn is_exempt_special_domain(url_str: &str) -> bool {
    let lower = url_str.trim().to_lowercase();
    let host = if let Ok(parsed) = reqwest::Url::parse(&lower) {
        parsed.host_str().unwrap_or("").to_string()
    } else {
        // Fallback simple string inspection
        lower
            .split('/')
            .nth(2)
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_string()
    };

    if host.is_empty() {
        return false;
    }

    // 1. Darknet / special decentralized TLDs (Onion websites etc.)
    let exempt_tlds = [
        ".onion",
        ".i2p",
        ".loki",
        ".b32.i2p",
        ".local",
        ".lan",
        ".home",
        ".internal",
        ".localhost",
        ".test",
        ".example",
        ".invalid",
    ];
    if exempt_tlds.iter().any(|ext| host.ends_with(ext)) {
        return true;
    }

    // 2. Localhost & Private/Loopback IP ranges
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "::1"
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.starts_with("fe80:")
        || host.starts_with("fc00:")
        || host.starts_with("fd00:")
    {
        return true;
    }

    // Check 172.16.x.x - 172.31.x.x
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some(second_octet_str) = rest.split('.').next() {
            if let Ok(second_octet) = second_octet_str.parse::<u8>() {
                if (16..=31).contains(&second_octet) {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_title() {
        let html = "<html><head><title>My Special Page</title></head><body>Hello</body></html>";
        assert_eq!(
            extract_html_title(html),
            Some("My Special Page".to_string())
        );
    }

    #[test]
    fn test_soft_404_detection() {
        assert!(is_soft_404(Some("404 Page Not Found"), None));
        assert!(is_soft_404(
            None,
            Some("Sorry, this domain expired recently.")
        ));
        assert!(!is_soft_404(
            Some("Welcome to Rust"),
            Some("Rust is a great language.")
        ));
    }

    #[test]
    fn test_onion_and_special_domain_exemption() {
        assert!(is_exempt_special_domain(
            "http://expyuz5wqqfdgah56...onion/path"
        ));
        assert!(is_exempt_special_domain("https://darkweb.onion"));
        assert!(is_exempt_special_domain("http://service.i2p/"));
        assert!(is_exempt_special_domain("http://192.168.1.1/admin"));
        assert!(!is_exempt_special_domain("https://www.google.com/"));
        assert!(!is_exempt_special_domain("https://github.com/"));
    }
}
