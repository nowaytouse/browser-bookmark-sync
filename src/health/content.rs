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

/// Interpret the response page's named YouTube player state, never arbitrary quoted JSON.
/// This is static analysis, not JavaScript execution; unsupported states remain review-only.
pub fn structured_content_error(response_url: &str, html_str: &str) -> Option<&'static str> {
    let url = reqwest::Url::parse(response_url).ok()?;
    let host = url.host_str()?;
    if !(host == "youtube.com"
        || host.ends_with(".youtube.com")
        || host == "youtube-nocookie.com"
        || host.ends_with(".youtube-nocookie.com"))
    {
        return None;
    }
    let segments: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let video_id = match segments.as_slice() {
        ["watch"] => {
            let ids: Vec<_> = url.query_pairs().filter(|(key, _)| key == "v").collect();
            if ids.len() != 1 {
                return None;
            }
            ids[0].1.to_string()
        }
        ["shorts" | "embed" | "live", id] => id.to_string(),
        _ => return None,
    };
    if video_id.len() != 11
        || !video_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }

    let document = Html::parse_document(html_str);
    let selector = Selector::parse("script:not([src])").ok()?;
    let mut result = None;
    for script in document.select(&selector) {
        if script.value().attr("nomodule").is_some() {
            continue;
        }
        if script
            .ancestors()
            .filter_map(scraper::ElementRef::wrap)
            .any(|parent| {
                matches!(
                    parent.value().name(),
                    "pre" | "code" | "template" | "noscript"
                )
            })
        {
            continue;
        }
        if script.value().attr("type").is_some_and(|kind| {
            !matches!(
                kind.trim().to_ascii_lowercase().as_str(),
                "" | "text/javascript" | "application/javascript"
            )
        }) {
            continue;
        }
        let text = script.text().collect::<String>();
        let text = text.trim();
        let assignment = [
            "var ytInitialPlayerResponse",
            "let ytInitialPlayerResponse",
            "const ytInitialPlayerResponse",
            "ytInitialPlayerResponse",
            "window[\"ytInitialPlayerResponse\"]",
            "window['ytInitialPlayerResponse']",
        ]
        .iter()
        .find_map(|prefix| {
            text.strip_prefix(prefix)
                .and_then(|rest| rest.trim_start().strip_prefix('='))
        });
        let Some(json) = assignment else {
            continue;
        };
        // More than one candidate may be overwritten at runtime; do not guess which wins.
        if result.is_some() {
            return Some("StructuredContentUncertain");
        }
        let mut stream = serde_json::Deserializer::from_str(json).into_iter::<PlayerResponse>();
        let state = match stream.next() {
            Some(Ok(state)) if matches!(json[stream.byte_offset()..].trim(), "" | ";") => state,
            _ => return Some("StructuredContentUncertain"),
        };
        if state
            .video_details
            .as_ref()
            .is_some_and(|details| details.video_id != video_id)
            || (state.playability_status.status != "OK"
                && state
                    .streaming_data
                    .as_ref()
                    .is_some_and(|data| !data.is_null()))
        {
            return Some("StructuredContentUncertain");
        }
        result = Some(state);
    }
    let state = result?;
    if state.playability_status.status == "OK" {
        return None;
    }
    let status = &state.playability_status;
    let mut reasons = Vec::new();
    if let Some(reason) = &status.reason {
        reasons.push(reason.clone());
    }
    if let Some(renderer) = status
        .error_screen
        .as_ref()
        .and_then(|screen| screen.player_error_message_renderer.as_ref())
    {
        for message in [&renderer.reason, &renderer.subreason]
            .into_iter()
            .flatten()
        {
            let Some(text) = player_message_text(message) else {
                return Some("StructuredContentUncertain");
            };
            reasons.push(text);
        }
    }
    let reasons: Vec<_> = reasons
        .iter()
        .map(|reason| {
            reason
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .collect();
    let restricted = [
        "private",
        "sign in",
        "log in",
        "age restricted",
        "age-restricted",
        "confirm your age",
        "country",
        "region",
        "premium",
        "members",
        "payment",
        "captcha",
        "try again",
        "temporar",
    ];
    if reasons
        .iter()
        .any(|reason| restricted.iter().any(|marker| reason.contains(marker)))
    {
        return Some("StructuredContentUncertain");
    }
    let explicit_removal = [
        "video has been removed",
        "this video has been removed",
        "video was deleted",
        "this video was deleted",
        "video does not exist",
        "this video does not exist",
    ]
    .iter()
    .any(|prefix| {
        reasons.iter().any(|reason| {
            reason.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.is_empty()
                    || suffix == "."
                    || suffix.starts_with(" by ")
                    || suffix.starts_with(" for ")
            })
        })
    });
    if matches!(status.status.as_str(), "ERROR" | "UNPLAYABLE") && explicit_removal {
        Some("ContentMissing")
    } else {
        Some("StructuredContentUncertain")
    }
}

// Typed known fields reject duplicate status/reason/identity keys rather than silently using the last.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerResponse {
    playability_status: PlayerStatus,
    video_details: Option<PlayerVideoDetails>,
    streaming_data: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerStatus {
    status: String,
    reason: Option<String>,
    error_screen: Option<PlayerErrorScreen>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerVideoDetails {
    video_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerErrorScreen {
    player_error_message_renderer: Option<PlayerErrorRenderer>,
}

#[derive(serde::Deserialize)]
struct PlayerErrorRenderer {
    reason: Option<PlayerMessage>,
    subreason: Option<PlayerMessage>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerMessage {
    simple_text: Option<String>,
    runs: Option<Vec<PlayerTextRun>>,
}

#[derive(serde::Deserialize)]
struct PlayerTextRun {
    text: String,
}

fn player_message_text(message: &PlayerMessage) -> Option<String> {
    match (&message.simple_text, &message.runs) {
        (Some(text), None) => Some(text.clone()),
        (None, Some(runs)) => Some(runs.iter().map(|run| run.text.as_str()).collect()),
        // Multiple encodings may contradict each other; none is arbitrarily preferred.
        _ => None,
    }
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
        assert_eq!(
            structured_content_error(
                "https://www.youtube.com/watch?v=abcdefghijk",
                r#"<script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}};</script>"#
            ),
            Some("ContentMissing")
        );
        assert_eq!(
            structured_content_error(
                "https://www.youtube.com/watch?v=abcdefghijk",
                r#"<script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"OK"},"reason":"Video unavailable help"};</script>"#
            ),
            None
        );
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

    #[test]
    fn accuracy_player_examples_and_unrelated_pages_are_not_removal_evidence() {
        let assignment = r#"var ytInitialPlayerResponse = {"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}};"#;
        for html in [
            format!("<pre>{assignment}</pre>"),
            format!("<code>{assignment}</code>"),
            format!("<!-- <script>{assignment}</script> -->"),
            format!("<script type='application/json'>{assignment}</script>"),
            format!("<template><script>{assignment}</script></template>"),
            format!("<script src='/example.js'>{assignment}</script>"),
            format!("<script nomodule>{assignment}</script>"),
            format!("<script>const example = `{assignment}`;</script>"),
            r#"<script>{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}}</script>"#.into(),
        ] {
            assert_eq!(structured_content_error("https://www.youtube.com/watch?v=abcdefghijk", &html), None, "{html}");
        }
        for url in [
            "https://example.com/watch?v=abcdefghijk",
            "https://notyoutube.com/watch?v=abcdefghijk",
            "https://youtube.com.example.com/watch?v=abcdefghijk",
            "https://www.youtube.com/help",
            "https://www.youtube.com/watch?v=abcdefghijk&v=zyxwvutsrqp",
        ] {
            assert_eq!(
                structured_content_error(url, &format!("<script>{assignment}</script>")),
                None,
                "{url}"
            );
        }
    }

    #[test]
    fn accuracy_player_state_keeps_restrictions_and_conflicts_review_only() {
        let url = "https://www.youtube.com/watch?v=abcdefghijk";
        for json in [
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video unavailable"}}"#,
            r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"This video is private"}}"#,
            r#"{"playabilityStatus":{"status":"UNPLAYABLE","reason":"Not available in your country"}}"#,
            r#"{"playabilityStatus":{"status":"AGE_CHECK_REQUIRED","reason":"Sign in to confirm your age"}}"#,
            r#"{"playabilityStatus":{"status":"UNPLAYABLE","reason":"Available to Premium members"}}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Try again later"},"help":"Video has been removed"}"#,
            r#"{"playabilityStatus":{"status":"ERROR"},"reason":"Video has been removed"}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"},"videoDetails":{"videoId":"zyxwvutsrqp"}}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"},"streamingData":{"formats":[{}]}}"#,
            r#"{"playabilityStatus":{"status":"ERROR","status":"UNPLAYABLE","reason":"Video has been removed"}}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video has been removed","errorScreen":{"playerErrorMessageRenderer":{"subreason":{"simpleText":"This video is private"}}}}}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video unavailable","errorScreen":{"playerErrorMessageRenderer":{"subreason":{"simpleText":"This video is private","simpleText":"Video has been removed"}}}}}"#,
            r#"{"playabilityStatus":{"status":"ERROR","reason":"Video unavailable","errorScreen":{"playerErrorMessageRenderer":{"subreason":{"simpleText":"Video has been removed","runs":[{"text":"This video is private"}]}}}}}"#,
        ] {
            assert_eq!(
                structured_content_error(
                    url,
                    &format!("<script>var ytInitialPlayerResponse = {json};</script>")
                ),
                Some("StructuredContentUncertain"),
                "{json}"
            );
        }
        let removed = r#"<script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"ERROR","reason":"Video has been removed"}};</script>"#;
        let valid = r#"<script>var ytInitialPlayerResponse = {"playabilityStatus":{"status":"OK"},"streamingData":{"formats":[{}]},"help":"Video has been removed"};</script>"#;
        assert_eq!(structured_content_error(url, valid), None);
        assert_eq!(
            structured_content_error(url, &format!("{removed}{valid}")),
            Some("StructuredContentUncertain")
        );
        for json in [
            r#"{ "playabilityStatus": { "reason": "Video has been removed", "status": "ERROR" } }"#,
            r#"{"playabilityStatus":{"status":"UNPLAYABLE","reason":"Video unavailable","errorScreen":{"playerErrorMessageRenderer":{"subreason":{"runs":[{"text":"This video has been "},{"text":"removed by the uploader"}]}}}},"videoDetails":{"videoId":"abcdefghijk"}}"#,
        ] {
            for url in [
                url,
                "https://m.youtube.com/shorts/abcdefghijk",
                "https://www.youtube-nocookie.com/embed/abcdefghijk",
            ] {
                assert_eq!(
                    structured_content_error(
                        url,
                        &format!("<script>var ytInitialPlayerResponse = {json};</script>")
                    ),
                    Some("ContentMissing"),
                    "{url}: {json}"
                );
            }
        }
    }
}
