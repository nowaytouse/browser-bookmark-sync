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

/// Extract the short, user-visible parts most likely to contain an unavailable message.
/// Script/style payloads are deliberately excluded to avoid matching dormant UI strings.
pub fn extract_content_text(html_str: &str) -> Option<String> {
    let document = Html::parse_document(html_str);
    let selector = Selector::parse("h1, h2, h3, p, [role='alert']").ok()?;
    let text = document
        .select(&selector)
        .flat_map(|element| element.text())
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.chars().take(4096).collect())
    }
}

/// Content-level checks are meaningful only for a deep path or query URL, not a homepage.
pub fn is_deep_content_url(url_str: &str) -> bool {
    reqwest::Url::parse(url_str)
        .is_ok_and(|url| !url.path().trim_matches('/').is_empty() || url.query().is_some())
}

/// Detect potential Soft 404 (e.g. 200 OK returning "Page Not Found", "404", "Domain Expired")
pub fn is_soft_404(title: Option<&str>, body_preview: Option<&str>) -> bool {
    let title_lower = title.unwrap_or("").trim().to_lowercase();
    let body_lower = body_preview.unwrap_or("").trim().to_lowercase();

    let markers = [
        "page not found",
        "404 page not found",
        "404 not found",
        "domain for sale",
        "domain expired",
        "this domain is for sale",
        "buy this domain",
        "page does not exist",
        "this page doesn't exist",
        "this page doesn’t exist",
        "content is unavailable",
        "content isn't available",
        "content isn’t available",
        "article not found",
        "post has been removed",
        "post was deleted",
        "video unavailable",
        "video is unavailable",
        "video has been removed",
        "this video isn't available",
        "this video isn’t available",
        "404 页面不存在",
        "页面找不到",
        "页面不存在",
        "内容不存在",
        "内容已删除",
        "内容已下架",
        "帖子不存在",
        "帖子已被删除",
        "文章不存在",
        "文章已删除",
        "视频不存在",
        "视频已删除",
        "视频不见了",
        "稿件不存在",
        "稿件已失效",
        "已被作者删除",
        "该微博已被删除",
        "没有知识存在的荒原",
    ];

    let title_matches = markers.iter().any(|marker| {
        title_lower == *marker
            || title_lower.starts_with(&format!("{marker} "))
            || title_lower.starts_with(&format!("{marker} -"))
            || title_lower.starts_with(&format!("{marker} |"))
            || title_lower.ends_with(&format!(" - {marker}"))
            || title_lower.ends_with(&format!(" | {marker}"))
    });
    let short_body_matches = body_lower.chars().count() <= 1500
        && markers.iter().any(|marker| body_lower.contains(marker));

    title_matches || short_body_matches
}

/// A parked/expired domain can return HTTP 200 even though the original site is gone.
pub fn is_parked_domain(title: Option<&str>, body_preview: Option<&str>) -> bool {
    let title_lower = title.unwrap_or("").trim().to_lowercase();
    let body_lower = body_preview.unwrap_or("").trim().to_lowercase();
    let markers = [
        "domain for sale",
        "domain expired",
        "this domain is for sale",
        "buy this domain",
        "该域名已过期",
        "此域名出售",
        "域名出售",
    ];
    markers.iter().any(|marker| title_lower.contains(marker))
        || (body_lower.chars().count() <= 1500
            && markers.iter().any(|marker| body_lower.contains(marker)))
}

/// Detect structured error payloads on dynamic video pages that still return HTTP 200.
pub fn is_structured_content_missing(html_str: &str) -> bool {
    let lower = html_str.to_lowercase();
    let unavailable_status = lower.contains("\"playabilitystatus\":{\"status\":\"error\"")
        || lower.contains("\"playabilitystatus\":{\"status\":\"unplayable\"");
    let removal_reason = [
        "video has been removed",
        "video unavailable",
        "video is unavailable",
        "video isn't available",
        "video isn’t available",
        "video does not exist",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    unavailable_status && removal_reason
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
        assert!(is_soft_404(Some("Video unavailable - YouTube"), None));
        assert!(is_soft_404(None, Some("抱歉，该帖子已被删除")));
        assert!(is_parked_domain(
            Some("Example.com - Domain for Sale"),
            None
        ));
        assert!(!is_soft_404(
            Some("How to design a Page Not Found screen"),
            Some(&"A normal long article about error-page design. ".repeat(80))
        ));
    }

    #[test]
    fn test_visible_content_and_structured_video_missing_detection() {
        let html = "<html><head><script>page not found</script></head><body><h1>内容已删除</h1><p>该帖子无法继续查看</p></body></html>";
        assert_eq!(
            extract_content_text(html),
            Some("内容已删除 该帖子无法继续查看".to_string())
        );
        assert!(is_structured_content_missing(
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}}"#
        ));
        assert!(!is_structured_content_missing(
            r#"{"playabilityStatus":{"status":"OK"},"reason":"Video unavailable help"}"#
        ));
        assert!(is_deep_content_url("https://example.com/posts/1"));
        assert!(!is_deep_content_url("https://example.com/"));
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
