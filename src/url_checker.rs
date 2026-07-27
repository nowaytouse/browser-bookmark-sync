//! URL有效性检查模块
//!
//! 通过代理和直连双网络验证收藏夹URL的有效性。
//! 判断逻辑：任一网络成功即有效，双网络都失败才判定为无效。

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// URL检查结果状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    /// 有效 - 任一网络返回成功(HTTP 2xx)
    Valid,
    /// 重定向 - HTTP 3xx
    Redirected,
    /// 需身份验证 - HTTP 401
    AuthenticationRequired,
    /// 防火墙/Bot拦截 - HTTP 403 / Cloudflare
    ForbiddenOrBotBlocked,
    /// 触发限流 - HTTP 429
    RateLimited,
    /// 资源未找到 - HTTP 404
    NotFound,
    /// 资源已被永久删除 - HTTP 410
    Gone,
    /// 服务器内部错误 - HTTP 5xx
    ServerError,
    /// DNS解析失败
    DnsError,
    /// TLS/SSL握手错误
    TlsError,
    /// 请求超时
    Timeout,
    /// TCP连接错误
    ConnectionError,
    /// 不支持的协议 Scheme
    UnsupportedScheme,
    /// 不确定 - 单网络模式失败或超时
    Uncertain,
    /// 简单无效（保留兼容）
    Invalid,
    /// 跳过 - 本地文件、javascript:等非HTTP URL
    Skipped,
}

impl ValidationStatus {
    /// 绝不误删判断：仅确定性资源不存在 (Invalid/NotFound/Gone) 允许自动清除建议
    pub fn should_auto_delete(&self) -> bool {
        matches!(
            self,
            ValidationStatus::Invalid | ValidationStatus::NotFound | ValidationStatus::Gone
        )
    }

    /// 根据 HTTP 状态码与结果分类
    pub fn classify_http_code(code: u16) -> Self {
        match code {
            200..=299 => ValidationStatus::Valid,
            300..=399 => ValidationStatus::Redirected,
            401 => ValidationStatus::AuthenticationRequired,
            403 => ValidationStatus::ForbiddenOrBotBlocked,
            404 => ValidationStatus::NotFound,
            410 => ValidationStatus::Gone,
            429 => ValidationStatus::RateLimited,
            500..=599 => ValidationStatus::ServerError,
            _ => ValidationStatus::Uncertain,
        }
    }

    /// 根据代理和直连结果判定最终状态
    pub fn determine(
        proxy_result: Option<&HttpResult>,
        direct_result: Option<&HttpResult>,
    ) -> Self {
        let proxy_success = proxy_result.map(|r| r.is_success()).unwrap_or(false);
        let direct_success = direct_result.map(|r| r.is_success()).unwrap_or(false);

        if proxy_success || direct_success {
            return ValidationStatus::Valid;
        }

        let proxy_blocked = proxy_result.map(|r| r.is_blocked()).unwrap_or(false);
        let direct_blocked = direct_result.map(|r| r.is_blocked()).unwrap_or(false);

        if proxy_blocked || direct_blocked {
            return ValidationStatus::Uncertain;
        }

        let proxy_failed = proxy_result.map(|r| r.is_failure()).unwrap_or(false);
        let direct_failed = direct_result.map(|r| r.is_failure()).unwrap_or(false);

        if proxy_failed && direct_failed {
            return ValidationStatus::Invalid;
        }

        ValidationStatus::Uncertain
    }
}

/// HTTP请求结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResult {
    /// HTTP状态码 (None表示连接失败)
    pub status_code: Option<u16>,
    /// 错误信息
    pub error: Option<String>,
    /// 响应延迟(毫秒)
    pub latency_ms: u64,
    /// 是否超时
    pub is_timeout: bool,
}

impl HttpResult {
    /// 创建成功结果
    pub fn success(status_code: u16, latency_ms: u64) -> Self {
        Self {
            status_code: Some(status_code),
            error: None,
            latency_ms,
            is_timeout: false,
        }
    }

    /// 创建失败结果
    pub fn failure(error: String, latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some(error),
            latency_ms,
            is_timeout: false,
        }
    }

    /// 创建超时结果
    pub fn timeout(latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some("Request timeout".to_string()),
            latency_ms,
            is_timeout: true,
        }
    }

    /// 判断是否成功 (HTTP 2xx/3xx)
    pub fn is_success(&self) -> bool {
        match self.status_code {
            Some(code) => (200..400).contains(&code),
            None => false,
        }
    }

    /// 判断是否失败 (仅 404/410 等明确的"不存在"状态码)
    /// 403/503/429 等可能是 CF 验证、WAF 拦截、限流，不算确定性失败
    pub fn is_failure(&self) -> bool {
        if self.is_timeout {
            return false; // 超时不算确定性失败
        }
        match self.status_code {
            Some(code) => {
                // 只有这些状态码才算"确定性失败"（资源真的不存在）
                // 404 Not Found - 页面不存在
                // 410 Gone - 资源已永久删除
                // 451 Unavailable For Legal Reasons - 法律原因不可用
                matches!(code, 404 | 410 | 451)
            }
            None => {
                // 连接错误需要检查是否是 DNS 解析失败（域名不存在）
                if let Some(ref err) = self.error {
                    let err_lower = err.to_lowercase();
                    // DNS 解析失败 = 域名不存在 = 确定性失败
                    // 连接拒绝/重置 = 服务器问题 = 不确定
                    err_lower.contains("dns")
                        || err_lower.contains("no such host")
                        || err_lower.contains("name or service not known")
                        || err_lower.contains("getaddrinfo")
                        || err_lower.contains("resolve")
                } else {
                    false
                }
            }
        }
    }

    /// 判断是否是"可能有效但被拦截"的状态码
    /// 这些状态码通常是 CF 验证、WAF、限流等，浏览器可能可以正常访问
    pub fn is_blocked(&self) -> bool {
        match self.status_code {
            Some(code) => {
                // 403 Forbidden - 可能是 CF 验证、WAF
                // 429 Too Many Requests - 限流
                // 503 Service Unavailable - CF 验证页面
                // 520-530 Cloudflare 特定错误
                matches!(code, 403 | 429 | 503 | 520..=530)
            }
            None => false,
        }
    }
}

/// 单个URL的检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlCheckResult {
    /// 被检查的URL
    pub url: String,
    /// 最终状态
    pub status: ValidationStatus,
    /// 代理网络检查结果
    pub proxy_result: Option<HttpResult>,
    /// 直连网络检查结果
    pub direct_result: Option<HttpResult>,
    /// 汇总错误信息
    pub error_message: Option<String>,
}

impl UrlCheckResult {
    /// 创建跳过结果
    pub fn skipped(url: String, reason: &str) -> Self {
        Self {
            url,
            status: ValidationStatus::Skipped,
            proxy_result: None,
            direct_result: None,
            error_message: Some(reason.to_string()),
        }
    }

    /// 从检查结果创建
    pub fn from_results(
        url: String,
        proxy_result: Option<HttpResult>,
        direct_result: Option<HttpResult>,
    ) -> Self {
        let status = ValidationStatus::determine(proxy_result.as_ref(), direct_result.as_ref());

        // 生成错误信息
        let error_message = if status == ValidationStatus::Invalid {
            let mut errors = Vec::new();
            if let Some(ref pr) = proxy_result {
                if let Some(ref e) = pr.error {
                    errors.push(format!("Proxy: {}", e));
                } else if let Some(code) = pr.status_code {
                    errors.push(format!("Proxy: HTTP {}", code));
                }
            }
            if let Some(ref dr) = direct_result {
                if let Some(ref e) = dr.error {
                    errors.push(format!("Direct: {}", e));
                } else if let Some(code) = dr.status_code {
                    errors.push(format!("Direct: HTTP {}", code));
                }
            }
            if errors.is_empty() {
                None
            } else {
                Some(errors.join("; "))
            }
        } else {
            None
        };

        Self {
            url,
            status,
            proxy_result,
            direct_result,
            error_message,
        }
    }
}

/// URL检查器配置
#[derive(Debug, Clone)]
pub struct CheckerConfig {
    /// 代理服务器URL (如 http://127.0.0.1:7890)
    pub proxy_url: Option<String>,
    /// 请求超时秒数
    pub timeout_secs: u64,
    /// 并发请求数
    pub concurrency: usize,
    /// 重试次数
    pub retry_count: u32,
}

impl Default for CheckerConfig {
    fn default() -> Self {
        Self {
            proxy_url: None,
            timeout_secs: 10,
            concurrency: 10,
            retry_count: 1,
        }
    }
}

/// 检查报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckReport {
    /// 总检查数
    pub total_checked: usize,
    /// 有效数量
    pub valid_count: usize,
    /// 无效数量
    pub invalid_count: usize,
    /// 不确定数量
    pub uncertain_count: usize,
    /// 跳过数量
    pub skipped_count: usize,
    /// 无效收藏夹详情
    pub invalid_urls: Vec<InvalidBookmark>,
    /// 检查耗时(秒)
    pub check_duration_secs: f64,
}

impl CheckReport {
    /// 从检查结果生成报告
    pub fn from_results(results: &[UrlCheckResult], duration_secs: f64) -> Self {
        let mut report = Self {
            total_checked: results.len(),
            valid_count: 0,
            invalid_count: 0,
            uncertain_count: 0,
            skipped_count: 0,
            invalid_urls: Vec::new(),
            check_duration_secs: duration_secs,
        };

        for result in results {
            match result.status {
                ValidationStatus::Valid => report.valid_count += 1,
                ValidationStatus::Invalid | ValidationStatus::NotFound | ValidationStatus::Gone => {
                    report.invalid_count += 1;
                    report.invalid_urls.push(InvalidBookmark {
                        title: String::new(),
                        url: result.url.clone(),
                        browser: String::new(),
                        folder_path: String::new(),
                        proxy_error: result.proxy_result.as_ref().and_then(|r| r.error.clone()),
                        direct_error: result.direct_result.as_ref().and_then(|r| r.error.clone()),
                    });
                }
                ValidationStatus::Skipped => report.skipped_count += 1,
                _ => report.uncertain_count += 1,
            }
        }

        report
    }

    /// 验证数量一致性 (属性8)
    pub fn is_consistent(&self) -> bool {
        self.valid_count + self.invalid_count + self.uncertain_count + self.skipped_count
            == self.total_checked
    }
}

/// 无效收藏夹详情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidBookmark {
    pub title: String,
    pub url: String,
    pub browser: String,
    pub folder_path: String,
    pub proxy_error: Option<String>,
    pub direct_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_status_both_success() {
        let proxy = HttpResult::success(200, 100);
        let direct = HttpResult::success(200, 50);
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), Some(&direct)),
            ValidationStatus::Valid
        );
    }

    #[test]
    fn test_validation_status_proxy_success_direct_fail() {
        let proxy = HttpResult::success(200, 100);
        let direct = HttpResult::success(404, 50);
        // 任一成功即有效
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), Some(&direct)),
            ValidationStatus::Valid
        );
    }

    #[test]
    fn test_validation_status_both_fail() {
        // 双网络都返回 404 = 确定性失败
        let proxy = HttpResult::success(404, 100);
        let direct = HttpResult::success(404, 50);
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), Some(&direct)),
            ValidationStatus::Invalid
        );
    }

    #[test]
    fn test_validation_status_cf_blocked() {
        // 403/503 是 CF 验证，应该标记为不确定
        let proxy = HttpResult::success(403, 100);
        let direct = HttpResult::success(503, 50);
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), Some(&direct)),
            ValidationStatus::Uncertain
        );
    }

    #[test]
    fn test_validation_status_single_network_fail() {
        let direct = HttpResult::success(404, 50);
        // 单网络失败应为不确定
        assert_eq!(
            ValidationStatus::determine(None, Some(&direct)),
            ValidationStatus::Uncertain
        );
    }

    #[test]
    fn test_http_result_is_success() {
        assert!(HttpResult::success(200, 100).is_success());
        assert!(HttpResult::success(301, 100).is_success());
        assert!(!HttpResult::success(404, 100).is_success());
        assert!(!HttpResult::failure("error".to_string(), 100).is_success());
    }

    #[test]
    fn test_check_report_consistency() {
        let results = vec![
            UrlCheckResult::from_results(
                "http://valid.com".to_string(),
                Some(HttpResult::success(200, 100)),
                Some(HttpResult::success(200, 50)),
            ),
            UrlCheckResult::skipped("javascript:void(0)".to_string(), "Non-HTTP URL"),
        ];
        let report = CheckReport::from_results(&results, 1.0);
        assert!(report.is_consistent());
        assert_eq!(report.total_checked, 2);
        assert_eq!(report.valid_count, 1);
        assert_eq!(report.skipped_count, 1);
    }
}

/// URL检查器
pub struct UrlChecker {
    config: CheckerConfig,
    /// 代理HTTP客户端 (None表示未配置代理)
    proxy_client: Option<Client>,
    /// 直连HTTP客户端
    direct_client: Client,
}

impl UrlChecker {
    /// 创建新的URL检查器
    pub fn new(config: CheckerConfig) -> Result<Self> {
        let timeout = Duration::from_secs(config.timeout_secs);
        let connect_timeout = Duration::from_secs(10); // 连接超时固定10秒

        // 创建直连客户端
        let direct_client = Client::builder()
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;

        // 创建代理客户端 (如果配置了代理)
        let proxy_client = if let Some(ref proxy_url) = config.proxy_url {
            info!("🌐 配置代理: {}", proxy_url);
            let proxy = reqwest::Proxy::all(proxy_url)?;
            Some(
                Client::builder()
                    .timeout(timeout)
                    .connect_timeout(connect_timeout)
                    .pool_idle_timeout(Duration::from_secs(30))
                    .pool_max_idle_per_host(10)
                    .proxy(proxy)
                    .user_agent(
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
                    )
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()?,
            )
        } else {
            debug!("未配置代理，仅使用直连模式");
            None
        };

        Ok(Self {
            config,
            proxy_client,
            direct_client,
        })
    }

    /// 检查URL是否应该跳过
    fn should_skip(url: &str) -> Option<&'static str> {
        let url_lower = url.to_lowercase();

        if url_lower.starts_with("javascript:") {
            return Some("JavaScript URL");
        }
        if url_lower.starts_with("data:") {
            return Some("Data URL");
        }
        if url_lower.starts_with("file://") {
            return Some("Local file");
        }
        if url_lower.starts_with("about:") {
            return Some("Browser internal URL");
        }
        if url_lower.starts_with("chrome://") || url_lower.starts_with("brave://") {
            return Some("Browser internal URL");
        }
        if url_lower.is_empty() {
            return Some("Empty URL");
        }
        // 跳过 .onion 地址（需要 Tor）
        if url_lower.contains(".onion") {
            return Some("Tor hidden service");
        }
        // 跳过本地地址
        if url_lower.contains("127.0.0.1") || url_lower.contains("localhost") {
            return Some("Local address");
        }

        None
    }

    /// 执行单个HTTP请求
    /// 先尝试 HEAD 请求，如果返回 405 则回退到 GET 请求
    async fn do_request(client: &Client, url: &str) -> HttpResult {
        let start = std::time::Instant::now();

        // 先尝试 HEAD 请求
        match client.head(url).send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                let latency = start.elapsed().as_millis() as u64;

                // 如果 HEAD 返回 405 (Method Not Allowed)，回退到 GET
                if status == 405 {
                    debug!("HEAD 返回 405，回退到 GET: {}", url);
                    return Self::do_get_request(client, url, start).await;
                }

                HttpResult::success(status, latency)
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                if e.is_timeout() {
                    HttpResult::timeout(latency)
                } else {
                    HttpResult::failure(e.to_string(), latency)
                }
            }
        }
    }

    /// 执行 GET 请求（仅获取响应头，不下载 body）
    async fn do_get_request(client: &Client, url: &str, start: std::time::Instant) -> HttpResult {
        match client.get(url).send().await {
            Ok(response) => {
                let latency = start.elapsed().as_millis() as u64;
                HttpResult::success(response.status().as_u16(), latency)
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                if e.is_timeout() {
                    HttpResult::timeout(latency)
                } else {
                    HttpResult::failure(e.to_string(), latency)
                }
            }
        }
    }

    /// 检查单个URL
    pub async fn check_url(&self, url: &str) -> UrlCheckResult {
        // 检查是否应该跳过
        if let Some(reason) = Self::should_skip(url) {
            return UrlCheckResult::skipped(url.to_string(), reason);
        }

        // 并行发起代理和直连请求
        let (proxy_result, direct_result) = if let Some(ref proxy_client) = self.proxy_client {
            let proxy_fut = Self::do_request(proxy_client, url);
            let direct_fut = Self::do_request(&self.direct_client, url);

            let (pr, dr) = tokio::join!(proxy_fut, direct_fut);
            (Some(pr), Some(dr))
        } else {
            // 无代理，仅直连
            let dr = Self::do_request(&self.direct_client, url).await;
            (None, Some(dr))
        };

        UrlCheckResult::from_results(url.to_string(), proxy_result, direct_result)
    }

    /// 批量检查URL - 使用流式处理，分批执行避免资源耗尽
    pub async fn check_batch<F>(
        &self,
        urls: Vec<String>,
        progress_callback: F,
    ) -> Vec<UrlCheckResult>
    where
        F: Fn(usize, usize, &str),
    {
        use futures::stream::{self, StreamExt};

        let total = urls.len();
        let concurrency = self.config.concurrency;

        // 使用 buffer_unordered 流式处理，限制并发数
        let results: Vec<UrlCheckResult> = stream::iter(urls.into_iter().enumerate())
            .map(|(i, url)| {
                let proxy_client = self.proxy_client.clone();
                let direct_client = self.direct_client.clone();

                async move {
                    let result = if let Some(reason) = Self::should_skip(&url) {
                        UrlCheckResult::skipped(url.clone(), reason)
                    } else {
                        let (proxy_result, direct_result) = if let Some(ref pc) = proxy_client {
                            let proxy_fut = Self::do_request(pc, &url);
                            let direct_fut = Self::do_request(&direct_client, &url);
                            let (pr, dr) = tokio::join!(proxy_fut, direct_fut);
                            (Some(pr), Some(dr))
                        } else {
                            let dr = Self::do_request(&direct_client, &url).await;
                            (None, Some(dr))
                        };
                        UrlCheckResult::from_results(url.clone(), proxy_result, direct_result)
                    };
                    (i, url, result)
                }
            })
            .buffer_unordered(concurrency)
            .inspect(|(i, url, _)| {
                progress_callback(*i + 1, total, url);
            })
            .map(|(_, _, result)| result)
            .collect()
            .await;

        results
    }

    /// 获取配置
    pub fn config(&self) -> &CheckerConfig {
        &self.config
    }

    /// 是否配置了代理
    pub fn has_proxy(&self) -> bool {
        self.proxy_client.is_some()
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // 生成HTTP状态码的策略
    fn success_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(200u16),
            Just(201u16),
            Just(204u16),
            Just(301u16),
            Just(302u16),
            Just(304u16),
        ]
    }

    // 确定性失败状态码（资源真的不存在）
    fn definite_failure_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(404u16), // Not Found
            Just(410u16), // Gone
            Just(451u16), // Unavailable For Legal Reasons
        ]
    }

    // 可能被拦截的状态码（CF 验证、WAF 等）
    fn blocked_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(403u16), // Forbidden (CF 验证)
            Just(429u16), // Too Many Requests
            Just(503u16), // Service Unavailable (CF 验证页面)
            Just(520u16), // Cloudflare error
        ]
    }

    fn any_latency() -> impl Strategy<Value = u64> {
        0u64..10000u64
    }

    /// **Feature: bookmark-validity-checker, Property 1: 任一网络成功即判定为有效**
    /// **Validates: Requirements 1.3**
    proptest! {
        #[test]
        fn prop_any_success_is_valid(
            proxy_code in success_status_code(),
            direct_code in definite_failure_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 代理成功，直连失败 -> 应该有效
            let proxy = HttpResult::success(proxy_code, latency1);
            let direct = HttpResult::success(direct_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Valid);
        }

        #[test]
        fn prop_direct_success_is_valid(
            proxy_code in definite_failure_status_code(),
            direct_code in success_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 代理失败，直连成功 -> 应该有效
            let proxy = HttpResult::success(proxy_code, latency1);
            let direct = HttpResult::success(direct_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Valid);
        }

        #[test]
        fn prop_both_success_is_valid(
            proxy_code in success_status_code(),
            direct_code in success_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都成功 -> 应该有效
            let proxy = HttpResult::success(proxy_code, latency1);
            let direct = HttpResult::success(direct_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Valid);
        }
    }

    /// **Feature: bookmark-validity-checker, Property 2: 双网络都返回404/410才判定为无效**
    /// **Validates: Requirements 1.4**
    proptest! {
        #[test]
        fn prop_both_definite_fail_is_invalid(
            proxy_code in definite_failure_status_code(),
            direct_code in definite_failure_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都返回 404/410 -> 应该无效（资源确实不存在）
            let proxy = HttpResult::success(proxy_code, latency1);
            let direct = HttpResult::success(direct_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Invalid);
        }

        #[test]
        fn prop_dns_error_is_invalid(
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都 DNS 解析失败 -> 应该无效（域名不存在）
            let proxy = HttpResult::failure("dns resolution failed".to_string(), latency1);
            let direct = HttpResult::failure("no such host".to_string(), latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Invalid);
        }
    }

    /// **Feature: bookmark-validity-checker, Property 9: CF/WAF 拦截标记为不确定**
    /// **Validates: Requirements 1.6 (新增)**
    proptest! {
        #[test]
        fn prop_blocked_is_uncertain(
            proxy_code in blocked_status_code(),
            direct_code in blocked_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都返回 403/503 -> 应该不确定（可能是 CF 验证）
            let proxy = HttpResult::success(proxy_code, latency1);
            let direct = HttpResult::success(direct_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Uncertain);
        }

        #[test]
        fn prop_one_blocked_is_uncertain(
            blocked_code in blocked_status_code(),
            fail_code in definite_failure_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 一个被拦截，一个 404 -> 应该不确定
            let proxy = HttpResult::success(blocked_code, latency1);
            let direct = HttpResult::success(fail_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Uncertain);
        }
    }

    /// **Feature: bookmark-validity-checker, Property 8: 报告数量一致性**
    /// **Validates: Requirements 4.2**
    proptest! {
        #[test]
        fn prop_report_consistency(
            valid_count in 0usize..100,
            invalid_count in 0usize..100,
            uncertain_count in 0usize..100,
            skipped_count in 0usize..100,
        ) {
            let mut results = Vec::new();

            // 生成有效结果
            for _ in 0..valid_count {
                results.push(UrlCheckResult::from_results(
                    "http://valid.com".to_string(),
                    Some(HttpResult::success(200, 100)),
                    Some(HttpResult::success(200, 50)),
                ));
            }

            // 生成无效结果
            for _ in 0..invalid_count {
                results.push(UrlCheckResult::from_results(
                    "http://invalid.com".to_string(),
                    Some(HttpResult::success(404, 100)),
                    Some(HttpResult::success(404, 50)),
                ));
            }

            // 生成不确定结果
            for _ in 0..uncertain_count {
                results.push(UrlCheckResult::from_results(
                    "http://uncertain.com".to_string(),
                    None,
                    Some(HttpResult::success(404, 50)),
                ));
            }

            // 生成跳过结果
            for _ in 0..skipped_count {
                results.push(UrlCheckResult::skipped(
                    "javascript:void(0)".to_string(),
                    "JavaScript URL",
                ));
            }

            let report = CheckReport::from_results(&results, 1.0);

            // 验证数量一致性
            prop_assert!(report.is_consistent());
            prop_assert_eq!(
                report.valid_count + report.invalid_count + report.uncertain_count + report.skipped_count,
                report.total_checked
            );
        }
    }
}

#[cfg(test)]
mod property_tests_2 {
    use super::*;
    use proptest::prelude::*;

    fn any_latency() -> impl Strategy<Value = u64> {
        0u64..10000u64
    }

    // 确定性失败状态码
    fn definite_failure_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![Just(404u16), Just(410u16),]
    }

    /// **Feature: bookmark-validity-checker, Property 7: 超时标记为不确定**
    /// **Validates: Requirements 5.2**
    proptest! {
        #[test]
        fn prop_timeout_is_uncertain(
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 代理超时，直连超时 -> 应该不确定（超时不算确定性失败）
            let proxy = HttpResult::timeout(latency1);
            let direct = HttpResult::timeout(latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            // 超时不算失败，所以不应该是Invalid
            prop_assert_ne!(status, ValidationStatus::Invalid);
        }

        #[test]
        fn prop_one_timeout_one_fail_is_uncertain(
            fail_code in definite_failure_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 代理超时，直连失败 -> 应该不确定
            let proxy = HttpResult::timeout(latency1);
            let direct = HttpResult::success(fail_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            // 一个超时一个失败，不应该判定为Invalid
            prop_assert_eq!(status, ValidationStatus::Uncertain);
        }
    }

    /// **Feature: bookmark-validity-checker, Property 3: 单网络模式失败判定为不确定**
    /// **Validates: Requirements 1.5**
    proptest! {
        #[test]
        fn prop_single_network_fail_is_uncertain(
            fail_code in definite_failure_status_code(),
            latency in any_latency(),
        ) {
            // 仅直连，且失败 -> 应该不确定
            let direct = HttpResult::success(fail_code, latency);
            let status = ValidationStatus::determine(None, Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Uncertain);
        }

        #[test]
        fn prop_single_network_error_is_uncertain(
            error in "[a-z]{5,20}",
            latency in any_latency(),
        ) {
            // 仅直连，且连接错误 -> 应该不确定
            let direct = HttpResult::failure(error, latency);
            let status = ValidationStatus::determine(None, Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Uncertain);
        }
    }
}

/// 删除操作结果
#[derive(Debug, Clone)]
pub struct DeleteResult {
    /// 删除的收藏夹数量
    pub deleted_count: usize,
    /// 保留的收藏夹数量（包括Uncertain）
    pub preserved_count: usize,
    /// 备份文件路径
    pub backup_path: Option<String>,
    /// 是否为dry-run模式
    pub is_dry_run: bool,
    /// 将被删除的URL列表（dry-run时使用）
    pub urls_to_delete: Vec<String>,
}

impl DeleteResult {
    /// 创建dry-run结果
    pub fn dry_run(urls_to_delete: Vec<String>, preserved_count: usize) -> Self {
        Self {
            deleted_count: urls_to_delete.len(),
            preserved_count,
            backup_path: None,
            is_dry_run: true,
            urls_to_delete,
        }
    }

    /// 创建实际删除结果
    pub fn actual(deleted_count: usize, preserved_count: usize, backup_path: String) -> Self {
        Self {
            deleted_count,
            preserved_count,
            backup_path: Some(backup_path),
            is_dry_run: false,
            urls_to_delete: Vec::new(),
        }
    }
}

/// 从收藏夹树中收集所有URL
pub fn collect_urls_from_bookmarks(bookmarks: &[crate::browsers::Bookmark]) -> Vec<String> {
    let mut urls = Vec::new();
    collect_urls_recursive(bookmarks, &mut urls);
    urls
}

fn collect_urls_recursive(bookmarks: &[crate::browsers::Bookmark], urls: &mut Vec<String>) {
    for bookmark in bookmarks {
        if bookmark.folder {
            collect_urls_recursive(&bookmark.children, urls);
        } else if let Some(ref url) = bookmark.url {
            urls.push(url.clone());
        }
    }
}

/// 从收藏夹树中删除指定URL的收藏夹
/// 返回删除的数量
pub fn remove_invalid_bookmarks(
    bookmarks: &mut Vec<crate::browsers::Bookmark>,
    invalid_urls: &std::collections::HashSet<String>,
) -> usize {
    let mut removed = 0;
    remove_invalid_recursive(bookmarks, invalid_urls, &mut removed);
    removed
}

fn remove_invalid_recursive(
    bookmarks: &mut Vec<crate::browsers::Bookmark>,
    invalid_urls: &std::collections::HashSet<String>,
    removed: &mut usize,
) {
    // 先递归处理子文件夹
    for bookmark in bookmarks.iter_mut() {
        if bookmark.folder {
            remove_invalid_recursive(&mut bookmark.children, invalid_urls, removed);
        }
    }

    // 然后删除当前层级的无效收藏夹
    let before_len = bookmarks.len();
    bookmarks.retain(|b| {
        if b.folder {
            true // 保留文件夹
        } else if let Some(ref url) = b.url {
            !invalid_urls.contains(url)
        } else {
            true // 保留没有URL的项
        }
    });
    *removed += before_len - bookmarks.len();
}

/// 从收藏夹树中提取指定状态的收藏夹（通用版本）
pub fn extract_bookmarks_by_status(
    bookmarks: &[crate::browsers::Bookmark],
    target_urls: &std::collections::HashSet<String>,
) -> Vec<crate::browsers::Bookmark> {
    let mut result = Vec::new();
    extract_by_status_recursive(bookmarks, target_urls, &mut result);
    result
}

fn extract_by_status_recursive(
    bookmarks: &[crate::browsers::Bookmark],
    target_urls: &std::collections::HashSet<String>,
    result: &mut Vec<crate::browsers::Bookmark>,
) {
    for bookmark in bookmarks {
        if bookmark.folder {
            extract_by_status_recursive(&bookmark.children, target_urls, result);
        } else if let Some(ref url) = bookmark.url {
            if target_urls.contains(url) {
                result.push(bookmark.clone());
            }
        }
    }
}

/// 从收藏夹树中提取无效的收藏夹（用于导出）- 兼容旧代码
pub fn extract_invalid_bookmarks(
    bookmarks: &[crate::browsers::Bookmark],
    invalid_urls: &std::collections::HashSet<String>,
) -> Vec<crate::browsers::Bookmark> {
    extract_bookmarks_by_status(bookmarks, invalid_urls)
}

fn extract_invalid_recursive(
    bookmarks: &[crate::browsers::Bookmark],
    invalid_urls: &std::collections::HashSet<String>,
    result: &mut Vec<crate::browsers::Bookmark>,
) {
    for bookmark in bookmarks {
        if bookmark.folder {
            // 递归处理子文件夹
            extract_invalid_recursive(&bookmark.children, invalid_urls, result);
        } else if let Some(ref url) = bookmark.url {
            if invalid_urls.contains(url) {
                result.push(bookmark.clone());
            }
        }
    }
}

/// 验证删除操作只删除Invalid状态的收藏夹
pub fn validate_delete_targets(
    results: &[UrlCheckResult],
    targets: &std::collections::HashSet<String>,
) -> bool {
    for result in results {
        let is_target = targets.contains(&result.url);
        let is_invalid = result.status == ValidationStatus::Invalid;

        // 如果是删除目标，必须是Invalid状态
        if is_target && !is_invalid {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use crate::browsers::Bookmark;
    use std::collections::HashSet;

    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        }
    }

    fn make_folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: None,
            date_modified: None,
        }
    }

    #[test]
    fn test_collect_urls() {
        let bookmarks = vec![
            make_bookmark("A", "http://a.com"),
            make_folder(
                "Folder",
                vec![
                    make_bookmark("B", "http://b.com"),
                    make_bookmark("C", "http://c.com"),
                ],
            ),
        ];

        let urls = collect_urls_from_bookmarks(&bookmarks);
        assert_eq!(urls.len(), 3);
        assert!(urls.contains(&"http://a.com".to_string()));
        assert!(urls.contains(&"http://b.com".to_string()));
        assert!(urls.contains(&"http://c.com".to_string()));
    }

    #[test]
    fn test_remove_invalid_bookmarks() {
        let mut bookmarks = vec![
            make_bookmark("Valid", "http://valid.com"),
            make_bookmark("Invalid", "http://invalid.com"),
            make_folder(
                "Folder",
                vec![
                    make_bookmark("Valid2", "http://valid2.com"),
                    make_bookmark("Invalid2", "http://invalid2.com"),
                ],
            ),
        ];

        let invalid: HashSet<String> = vec![
            "http://invalid.com".to_string(),
            "http://invalid2.com".to_string(),
        ]
        .into_iter()
        .collect();

        let removed = remove_invalid_bookmarks(&mut bookmarks, &invalid);

        assert_eq!(removed, 2);
        assert_eq!(bookmarks.len(), 2); // Valid + Folder
        assert_eq!(bookmarks[1].children.len(), 1); // Only Valid2
    }
}

#[cfg(test)]
mod property_tests_3 {
    use super::*;
    use crate::browsers::Bookmark;
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        }
    }

    /// **Feature: bookmark-validity-checker, Property 5: 仅删除高置信度无效项**
    /// **Validates: Requirements 3.1**
    proptest! {
        #[test]
        fn prop_only_delete_invalid(
            valid_count in 1usize..10,
            invalid_count in 1usize..10,
            uncertain_count in 1usize..10,
        ) {
            let mut results = Vec::new();
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            // 生成有效收藏夹
            for i in 0..valid_count {
                let url = format!("http://valid{}.com", i);
                bookmarks.push(make_bookmark(&format!("Valid{}", i), &url));
                results.push(UrlCheckResult::from_results(
                    url,
                    Some(HttpResult::success(200, 100)),
                    Some(HttpResult::success(200, 50)),
                ));
            }

            // 生成无效收藏夹
            for i in 0..invalid_count {
                let url = format!("http://invalid{}.com", i);
                bookmarks.push(make_bookmark(&format!("Invalid{}", i), &url));
                results.push(UrlCheckResult::from_results(
                    url.clone(),
                    Some(HttpResult::success(404, 100)),
                    Some(HttpResult::success(404, 50)),
                ));
                invalid_urls.insert(url);
            }

            // 生成不确定收藏夹
            for i in 0..uncertain_count {
                let url = format!("http://uncertain{}.com", i);
                bookmarks.push(make_bookmark(&format!("Uncertain{}", i), &url));
                results.push(UrlCheckResult::from_results(
                    url,
                    None,
                    Some(HttpResult::success(404, 50)),
                ));
            }

            let original_count = bookmarks.len();
            let removed = remove_invalid_bookmarks(&mut bookmarks, &invalid_urls);

            // 验证：只删除了Invalid状态的收藏夹
            prop_assert_eq!(removed, invalid_count);
            // 验证：Valid和Uncertain都被保留
            prop_assert_eq!(bookmarks.len(), valid_count + uncertain_count);
            // 验证：删除目标都是Invalid状态
            prop_assert!(validate_delete_targets(&results, &invalid_urls));
        }
    }

    /// **Feature: bookmark-validity-checker, Property 6: Dry-run模式不修改数据**
    /// **Validates: Requirements 3.4**
    proptest! {
        #[test]
        fn prop_dry_run_no_modification(
            bookmark_count in 1usize..20,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for i in 0..bookmark_count {
                let url = format!("http://test{}.com", i);
                bookmarks.push(make_bookmark(&format!("Test{}", i), &url));
                if i % 2 == 0 {
                    invalid_urls.insert(url);
                }
            }

            // 克隆原始数据
            let original_bookmarks = bookmarks.clone();

            // 模拟dry-run：只计算要删除的，不实际删除
            let urls_to_delete: Vec<String> = invalid_urls.iter().cloned().collect();
            let _dry_run_result = DeleteResult::dry_run(
                urls_to_delete,
                bookmark_count - invalid_urls.len(),
            );

            // 验证：原始数据未被修改
            prop_assert_eq!(bookmarks.len(), original_bookmarks.len());
            for (a, b) in bookmarks.iter().zip(original_bookmarks.iter()) {
                prop_assert_eq!(&a.url, &b.url);
                prop_assert_eq!(&a.title, &b.title);
            }
        }
    }
}

// ============================================================
// 结构保持删除模块 - 保持文件夹层级关系
// ============================================================

/// 删除配置
#[derive(Debug, Clone)]
pub struct RemoveConfig {
    /// 是否保留空文件夹（默认 false = 删除空文件夹）
    pub keep_empty_folders: bool,
}

impl Default for RemoveConfig {
    fn default() -> Self {
        Self {
            keep_empty_folders: false,
        } // 默认删除空文件夹
    }
}

/// 删除结果统计
#[derive(Debug, Clone, Default)]
pub struct RemoveStats {
    /// 删除的书签数量
    pub bookmarks_removed: usize,
    /// 保留的书签数量
    pub bookmarks_preserved: usize,
    /// 删除的空文件夹数量
    pub empty_folders_removed: usize,
    /// 保留的文件夹数量
    pub folders_preserved: usize,
}

impl RemoveStats {
    /// 打印统计摘要
    pub fn print_summary(&self) {
        println!("\n📊 删除统计:");
        println!("  书签删除: {}", self.bookmarks_removed);
        println!("  书签保留: {}", self.bookmarks_preserved);
        println!("  空文件夹删除: {}", self.empty_folders_removed);
        println!("  文件夹保留: {}", self.folders_preserved);
    }
}

/// 从收藏夹树中删除指定URL的书签（保持文件夹结构）
///
/// 两阶段处理：
/// 1. 先删除书签项（保留所有文件夹）
/// 2. 根据配置决定是否清理空文件夹
pub fn remove_invalid_bookmarks_preserve_structure(
    bookmarks: &mut Vec<crate::browsers::Bookmark>,
    invalid_urls: &std::collections::HashSet<String>,
    config: &RemoveConfig,
) -> RemoveStats {
    let mut stats = RemoveStats::default();

    // 阶段1: 递归删除死链书签（保持所有文件夹）
    remove_bookmarks_only(bookmarks, invalid_urls, &mut stats);

    // 阶段2: 如果不保留空文件夹，则清理
    if !config.keep_empty_folders {
        cleanup_empty_folders_recursive(bookmarks, &mut stats);
    } else {
        // 统计保留的文件夹数量
        count_folders_recursive(bookmarks, &mut stats.folders_preserved);
    }

    stats
}

/// 只删除书签项，保留所有文件夹
fn remove_bookmarks_only(
    bookmarks: &mut Vec<crate::browsers::Bookmark>,
    invalid_urls: &std::collections::HashSet<String>,
    stats: &mut RemoveStats,
) {
    // 先递归处理所有子文件夹
    for bookmark in bookmarks.iter_mut() {
        if bookmark.folder {
            remove_bookmarks_only(&mut bookmark.children, invalid_urls, stats);
        }
    }

    // 只删除书签项，保留所有文件夹
    bookmarks.retain(|b| {
        if b.folder {
            true // 始终保留文件夹
        } else if let Some(ref url) = b.url {
            if invalid_urls.contains(url) {
                stats.bookmarks_removed += 1;
                false // 删除死链
            } else {
                stats.bookmarks_preserved += 1;
                true // 保留有效书签
            }
        } else {
            stats.bookmarks_preserved += 1;
            true // 保留没有URL的项
        }
    });
}

/// 从叶子向上递归清理空文件夹
fn cleanup_empty_folders_recursive(
    bookmarks: &mut Vec<crate::browsers::Bookmark>,
    stats: &mut RemoveStats,
) {
    // 先递归处理子文件夹
    for bookmark in bookmarks.iter_mut() {
        if bookmark.folder {
            cleanup_empty_folders_recursive(&mut bookmark.children, stats);
        }
    }

    // 删除空文件夹
    bookmarks.retain(|b| {
        if b.folder && b.children.is_empty() {
            stats.empty_folders_removed += 1;
            false // 删除空文件夹
        } else {
            if b.folder {
                stats.folders_preserved += 1;
            }
            true
        }
    });
}

/// 统计文件夹数量
fn count_folders_recursive(bookmarks: &[crate::browsers::Bookmark], count: &mut usize) {
    for bookmark in bookmarks {
        if bookmark.folder {
            *count += 1;
            count_folders_recursive(&bookmark.children, count);
        }
    }
}

/// 按状态提取书签并保持文件夹结构
pub fn extract_by_status_preserve_structure(
    bookmarks: &[crate::browsers::Bookmark],
    target_urls: &std::collections::HashSet<String>,
) -> Vec<crate::browsers::Bookmark> {
    extract_preserve_structure_recursive(bookmarks, target_urls)
}

/// 递归提取，保持文件夹层级
fn extract_preserve_structure_recursive(
    bookmarks: &[crate::browsers::Bookmark],
    target_urls: &std::collections::HashSet<String>,
) -> Vec<crate::browsers::Bookmark> {
    let mut result = Vec::new();

    for bookmark in bookmarks {
        if bookmark.folder {
            // 递归处理子文件夹
            let children = extract_preserve_structure_recursive(&bookmark.children, target_urls);
            if !children.is_empty() {
                // 只有当子文件夹有内容时才保留该文件夹
                let mut folder = bookmark.clone();
                folder.children = children;
                result.push(folder);
            }
        } else if let Some(ref url) = bookmark.url {
            if target_urls.contains(url) {
                result.push(bookmark.clone());
            }
        }
    }

    result
}

/// 获取书签的完整路径（用于验证）
pub fn get_bookmark_path(
    bookmarks: &[crate::browsers::Bookmark],
    target_url: &str,
) -> Option<Vec<String>> {
    fn find_path(
        bookmarks: &[crate::browsers::Bookmark],
        target_url: &str,
        current_path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        for bookmark in bookmarks {
            if bookmark.folder {
                current_path.push(bookmark.title.clone());
                if let Some(path) = find_path(&bookmark.children, target_url, current_path) {
                    return Some(path);
                }
                current_path.pop();
            } else if let Some(ref url) = bookmark.url {
                if url == target_url {
                    return Some(current_path.clone());
                }
            }
        }
        None
    }

    let mut path = Vec::new();
    find_path(bookmarks, target_url, &mut path)
}

/// 收集所有书签的路径（用于验证）
pub fn collect_all_bookmark_paths(
    bookmarks: &[crate::browsers::Bookmark],
) -> std::collections::HashMap<String, Vec<String>> {
    let mut paths = std::collections::HashMap::new();
    collect_paths_recursive(bookmarks, &mut Vec::new(), &mut paths);
    paths
}

fn collect_paths_recursive(
    bookmarks: &[crate::browsers::Bookmark],
    current_path: &mut Vec<String>,
    paths: &mut std::collections::HashMap<String, Vec<String>>,
) {
    for bookmark in bookmarks {
        if bookmark.folder {
            current_path.push(bookmark.title.clone());
            collect_paths_recursive(&bookmark.children, current_path, paths);
            current_path.pop();
        } else if let Some(ref url) = bookmark.url {
            paths.insert(url.clone(), current_path.clone());
        }
    }
}

/// 检查是否存在空文件夹
pub fn has_empty_folders(bookmarks: &[crate::browsers::Bookmark]) -> bool {
    for bookmark in bookmarks {
        if bookmark.folder {
            if bookmark.children.is_empty() {
                return true;
            }
            if has_empty_folders(&bookmark.children) {
                return true;
            }
        }
    }
    false
}

/// 统计书签总数
pub fn count_bookmarks(bookmarks: &[crate::browsers::Bookmark]) -> usize {
    let mut count = 0;
    for bookmark in bookmarks {
        if bookmark.folder {
            count += count_bookmarks(&bookmark.children);
        } else {
            count += 1;
        }
    }
    count
}

/// 统计文件夹总数
pub fn count_folders(bookmarks: &[crate::browsers::Bookmark]) -> usize {
    let mut count = 0;
    for bookmark in bookmarks {
        if bookmark.folder {
            count += 1;
            count += count_folders(&bookmark.children);
        }
    }
    count
}

#[cfg(test)]
mod structure_preserve_tests {
    use super::*;
    use crate::browsers::Bookmark;
    use std::collections::HashSet;

    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        }
    }

    fn make_folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: None,
            date_modified: None,
        }
    }

    #[test]
    fn test_preserve_structure_basic() {
        // 创建嵌套结构: Root/AI工具/ChatGPT, Root/AI工具/Claude
        let mut bookmarks = vec![
            make_folder(
                "AI工具",
                vec![
                    make_bookmark("ChatGPT", "http://chatgpt.com"),
                    make_bookmark("Claude", "http://claude.ai"),
                ],
            ),
            make_folder(
                "开发工具",
                vec![
                    make_bookmark("GitHub", "http://github.com"),
                    make_bookmark("DeadLink", "http://dead.link"),
                ],
            ),
        ];

        let invalid: HashSet<String> = vec!["http://dead.link".to_string()].into_iter().collect();
        let config = RemoveConfig {
            keep_empty_folders: true,
        };

        let stats = remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid, &config);

        assert_eq!(stats.bookmarks_removed, 1);
        assert_eq!(stats.bookmarks_preserved, 3);
        // 文件夹结构应该保持
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].title, "AI工具");
        assert_eq!(bookmarks[1].title, "开发工具");
        assert_eq!(bookmarks[1].children.len(), 1); // 只剩 GitHub
    }

    #[test]
    fn test_default_removes_empty_folders() {
        // 创建一个文件夹，其中所有书签都是死链
        let mut bookmarks = vec![
            make_folder(
                "全是死链",
                vec![
                    make_bookmark("Dead1", "http://dead1.com"),
                    make_bookmark("Dead2", "http://dead2.com"),
                ],
            ),
            make_folder(
                "有效文件夹",
                vec![make_bookmark("Valid", "http://valid.com")],
            ),
        ];

        let invalid: HashSet<String> = vec![
            "http://dead1.com".to_string(),
            "http://dead2.com".to_string(),
        ]
        .into_iter()
        .collect();

        let config = RemoveConfig::default(); // keep_empty_folders = false
        let stats = remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid, &config);

        assert_eq!(stats.bookmarks_removed, 2);
        assert_eq!(stats.empty_folders_removed, 1);
        assert_eq!(bookmarks.len(), 1); // 只剩"有效文件夹"
        assert_eq!(bookmarks[0].title, "有效文件夹");
    }

    #[test]
    fn test_keep_empty_folders() {
        let mut bookmarks = vec![make_folder(
            "全是死链",
            vec![make_bookmark("Dead1", "http://dead1.com")],
        )];

        let invalid: HashSet<String> = vec!["http://dead1.com".to_string()].into_iter().collect();
        let config = RemoveConfig {
            keep_empty_folders: true,
        };

        let stats = remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid, &config);

        assert_eq!(stats.bookmarks_removed, 1);
        assert_eq!(stats.empty_folders_removed, 0);
        assert_eq!(bookmarks.len(), 1); // 空文件夹保留
        assert!(bookmarks[0].children.is_empty());
    }

    #[test]
    fn test_nested_empty_folders_cleanup() {
        // 嵌套空文件夹: A/B/C，C中的书签全是死链
        let mut bookmarks = vec![make_folder(
            "A",
            vec![make_folder(
                "B",
                vec![make_folder(
                    "C",
                    vec![make_bookmark("Dead", "http://dead.com")],
                )],
            )],
        )];

        let invalid: HashSet<String> = vec!["http://dead.com".to_string()].into_iter().collect();
        let config = RemoveConfig::default();

        let stats = remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid, &config);

        assert_eq!(stats.bookmarks_removed, 1);
        assert_eq!(stats.empty_folders_removed, 3); // A, B, C 都被删除
        assert!(bookmarks.is_empty());
    }

    #[test]
    fn test_path_preserved() {
        let mut bookmarks = vec![make_folder(
            "工具",
            vec![make_folder(
                "AI",
                vec![
                    make_bookmark("ChatGPT", "http://chatgpt.com"),
                    make_bookmark("Dead", "http://dead.com"),
                ],
            )],
        )];

        // 记录删除前的路径
        let path_before = get_bookmark_path(&bookmarks, "http://chatgpt.com");
        assert_eq!(
            path_before,
            Some(vec!["工具".to_string(), "AI".to_string()])
        );

        let invalid: HashSet<String> = vec!["http://dead.com".to_string()].into_iter().collect();
        let config = RemoveConfig {
            keep_empty_folders: true,
        };
        remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid, &config);

        // 验证路径不变
        let path_after = get_bookmark_path(&bookmarks, "http://chatgpt.com");
        assert_eq!(path_before, path_after);
    }
}

#[cfg(test)]
mod property_tests_structure {
    use super::*;
    use crate::browsers::Bookmark;
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        }
    }

    fn make_folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: None,
            date_modified: None,
        }
    }

    /// **Feature: folder-structure-preservation, Property 1: 文件夹层级关系保持**
    /// **Validates: Requirements 1.1, 1.3**
    proptest! {
        #[test]
        fn prop_folder_path_preserved(
            valid_count in 1usize..5,
            invalid_count in 0usize..3,
        ) {
            // 创建简单的嵌套结构
            let mut bookmarks_in_folder = Vec::new();
            let mut all_urls = Vec::new();
            let mut invalid_urls = HashSet::new();

            for i in 0..valid_count {
                let url = format!("http://valid{}.com", i);
                bookmarks_in_folder.push(make_bookmark(&format!("Valid{}", i), &url));
                all_urls.push(url);
            }

            for i in 0..invalid_count {
                let url = format!("http://invalid{}.com", i);
                bookmarks_in_folder.push(make_bookmark(&format!("Invalid{}", i), &url));
                invalid_urls.insert(url);
            }

            let mut bookmarks = vec![
                make_folder("TestFolder", bookmarks_in_folder),
            ];

            // 记录删除前的路径
            let paths_before = collect_all_bookmark_paths(&bookmarks);

            let config = RemoveConfig { keep_empty_folders: true };
            remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid_urls, &config);

            // 验证所有保留书签的路径不变
            let paths_after = collect_all_bookmark_paths(&bookmarks);
            for (url, path) in paths_after {
                if let Some(original_path) = paths_before.get(&url) {
                    prop_assert_eq!(&path, original_path, "Path changed for {}", url);
                }
            }
        }
    }

    /// **Feature: folder-structure-preservation, Property 5: 只删除死链书签**
    /// **Validates: Requirements 1.1**
    proptest! {
        #[test]
        fn prop_only_delete_dead_links(
            valid_count in 1usize..10,
            invalid_count in 1usize..5,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();
            let mut valid_urls = HashSet::new();

            for i in 0..valid_count {
                let url = format!("http://valid{}.com", i);
                bookmarks.push(make_bookmark(&format!("Valid{}", i), &url));
                valid_urls.insert(url);
            }

            for i in 0..invalid_count {
                let url = format!("http://invalid{}.com", i);
                bookmarks.push(make_bookmark(&format!("Invalid{}", i), &url));
                invalid_urls.insert(url);
            }

            let config = RemoveConfig { keep_empty_folders: true };
            let stats = remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid_urls, &config);

            // 验证删除数量正确
            prop_assert_eq!(stats.bookmarks_removed, invalid_count);
            prop_assert_eq!(stats.bookmarks_preserved, valid_count);

            // 验证所有有效URL都保留
            for bookmark in &bookmarks {
                if let Some(ref url) = bookmark.url {
                    prop_assert!(valid_urls.contains(url), "Valid URL was deleted: {}", url);
                    prop_assert!(!invalid_urls.contains(url), "Invalid URL was not deleted: {}", url);
                }
            }
        }
    }

    /// **Feature: folder-structure-preservation, Property 2: 默认模式删除空文件夹**
    /// **Validates: Requirements 2.1**
    proptest! {
        #[test]
        fn prop_no_empty_folders_default(
            folder_count in 1usize..3,
            bookmarks_per_folder in 1usize..3,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for f in 0..folder_count {
                let mut children = Vec::new();
                for b in 0..bookmarks_per_folder {
                    let url = format!("http://folder{}_bookmark{}.com", f, b);
                    children.push(make_bookmark(&format!("B{}_{}", f, b), &url));
                    // 标记所有书签为死链
                    invalid_urls.insert(url);
                }
                bookmarks.push(make_folder(&format!("Folder{}", f), children));
            }

            let config = RemoveConfig::default(); // keep_empty_folders = false
            remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid_urls, &config);

            // 验证没有空文件夹
            prop_assert!(!has_empty_folders(&bookmarks));
        }
    }

    /// **Feature: folder-structure-preservation, Property 3: 保留空文件夹模式**
    /// **Validates: Requirements 2.2**
    proptest! {
        #[test]
        fn prop_keep_empty_folders(
            folder_count in 1usize..3,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for f in 0..folder_count {
                let url = format!("http://dead{}.com", f);
                let children = vec![make_bookmark(&format!("Dead{}", f), &url)];
                bookmarks.push(make_folder(&format!("Folder{}", f), children));
                invalid_urls.insert(url);
            }

            let folders_before = count_folders(&bookmarks);

            let config = RemoveConfig { keep_empty_folders: true };
            remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid_urls, &config);

            let folders_after = count_folders(&bookmarks);

            // 验证文件夹数量不变
            prop_assert_eq!(folders_before, folders_after);
        }
    }

    /// **Feature: folder-structure-preservation, Property 4: 报告数量一致性**
    /// **Validates: Requirements 1.4**
    proptest! {
        #[test]
        fn prop_report_consistency(
            valid_count in 1usize..10,
            invalid_count in 0usize..5,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for i in 0..valid_count {
                let url = format!("http://valid{}.com", i);
                bookmarks.push(make_bookmark(&format!("Valid{}", i), &url));
            }

            for i in 0..invalid_count {
                let url = format!("http://invalid{}.com", i);
                bookmarks.push(make_bookmark(&format!("Invalid{}", i), &url));
                invalid_urls.insert(url);
            }

            let total_before = count_bookmarks(&bookmarks);

            let config = RemoveConfig { keep_empty_folders: true };
            let stats = remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid_urls, &config);

            // 验证数量一致性
            prop_assert_eq!(
                stats.bookmarks_removed + stats.bookmarks_preserved,
                total_before,
                "Report inconsistent: removed={}, preserved={}, total_before={}",
                stats.bookmarks_removed, stats.bookmarks_preserved, total_before
            );
        }
    }
}

#[cfg(test)]
mod extract_preserve_tests {
    use super::*;
    use crate::browsers::Bookmark;
    use std::collections::HashSet;

    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        }
    }

    fn make_folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: None,
            date_modified: None,
        }
    }

    #[test]
    fn test_extract_preserve_structure() {
        let bookmarks = vec![make_folder(
            "工具",
            vec![
                make_folder(
                    "AI",
                    vec![
                        make_bookmark("ChatGPT", "http://chatgpt.com"),
                        make_bookmark("Claude", "http://claude.ai"),
                    ],
                ),
                make_folder("开发", vec![make_bookmark("GitHub", "http://github.com")]),
            ],
        )];

        let target: HashSet<String> = vec!["http://chatgpt.com".to_string()].into_iter().collect();
        let extracted = extract_by_status_preserve_structure(&bookmarks, &target);

        // 验证结构保持
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].title, "工具");
        assert_eq!(extracted[0].children.len(), 1);
        assert_eq!(extracted[0].children[0].title, "AI");
        assert_eq!(extracted[0].children[0].children.len(), 1);
        assert_eq!(extracted[0].children[0].children[0].title, "ChatGPT");
    }

    #[test]
    fn test_extract_multiple_from_same_folder() {
        let bookmarks = vec![make_folder(
            "AI",
            vec![
                make_bookmark("ChatGPT", "http://chatgpt.com"),
                make_bookmark("Claude", "http://claude.ai"),
                make_bookmark("Gemini", "http://gemini.google.com"),
            ],
        )];

        let target: HashSet<String> = vec![
            "http://chatgpt.com".to_string(),
            "http://claude.ai".to_string(),
        ]
        .into_iter()
        .collect();

        let extracted = extract_by_status_preserve_structure(&bookmarks, &target);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].children.len(), 2);
    }

    #[test]
    fn test_extract_empty_result() {
        let bookmarks = vec![make_folder(
            "AI",
            vec![make_bookmark("ChatGPT", "http://chatgpt.com")],
        )];

        let target: HashSet<String> = vec!["http://nonexistent.com".to_string()]
            .into_iter()
            .collect();
        let extracted = extract_by_status_preserve_structure(&bookmarks, &target);

        assert!(extracted.is_empty());
    }
}

#[cfg(test)]
mod property_tests_extract {
    use super::*;
    use crate::browsers::Bookmark;
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn make_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: Some(url.to_string()),
            folder: false,
            children: vec![],
            date_added: None,
            date_modified: None,
        }
    }

    fn make_folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: title.to_string(),
            title: title.to_string(),
            url: None,
            folder: true,
            children,
            date_added: None,
            date_modified: None,
        }
    }

    /// **Feature: folder-structure-preservation, Property 6: 导出保持结构**
    /// **Validates: Requirements 6.1**
    proptest! {
        #[test]
        fn prop_extract_preserves_path(
            bookmark_count in 1usize..5,
            target_ratio in 0.2f64..0.8,
        ) {
            // 创建嵌套结构
            let mut children = Vec::new();
            let mut all_urls = Vec::new();

            for i in 0..bookmark_count {
                let url = format!("http://test{}.com", i);
                children.push(make_bookmark(&format!("Test{}", i), &url));
                all_urls.push(url);
            }

            let bookmarks = vec![
                make_folder("Parent", vec![
                    make_folder("Child", children),
                ]),
            ];

            // 选择部分URL作为目标
            let target_count = ((bookmark_count as f64) * target_ratio).max(1.0) as usize;
            let target_urls: HashSet<String> = all_urls.into_iter().take(target_count).collect();

            // 记录原始路径
            let original_paths = collect_all_bookmark_paths(&bookmarks);

            // 提取
            let extracted = extract_by_status_preserve_structure(&bookmarks, &target_urls);

            // 验证提取结果中的路径与原始路径一致
            let extracted_paths = collect_all_bookmark_paths(&extracted);
            for (url, path) in extracted_paths {
                if let Some(original_path) = original_paths.get(&url) {
                    prop_assert_eq!(&path, original_path, "Path changed for {}", url);
                }
            }
        }
    }

    /// **Feature: folder-structure-preservation, Property 8: 路径可达性**
    /// **Validates: Requirements 5.1**
    proptest! {
        #[test]
        fn prop_all_extracted_reachable(
            bookmark_count in 1usize..5,
        ) {
            let mut children = Vec::new();
            let mut target_urls = HashSet::new();

            for i in 0..bookmark_count {
                let url = format!("http://test{}.com", i);
                children.push(make_bookmark(&format!("Test{}", i), &url));
                target_urls.insert(url);
            }

            let bookmarks = vec![
                make_folder("Root", children),
            ];

            let extracted = extract_by_status_preserve_structure(&bookmarks, &target_urls);

            // 验证所有目标URL都能在提取结果中找到
            let extracted_paths = collect_all_bookmark_paths(&extracted);
            for url in &target_urls {
                prop_assert!(
                    extracted_paths.contains_key(url),
                    "URL {} not reachable in extracted result", url
                );
            }
        }
    }
}
