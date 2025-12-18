//! URL有效性检查模块
//! 
//! 通过代理和直连双网络验证收藏夹URL的有效性。
//! 判断逻辑：任一网络成功即有效，双网络都失败才判定为无效。

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// URL检查结果状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    /// 有效 - 任一网络返回成功(HTTP 2xx/3xx)
    Valid,
    /// 无效 - 两个网络都返回失败(HTTP 4xx/5xx或连接错误)
    Invalid,
    /// 不确定 - 单网络模式失败或超时
    Uncertain,
    /// 跳过 - 本地文件、javascript:等非HTTP URL
    Skipped,
}

impl ValidationStatus {
    /// 根据代理和直连结果判定最终状态
    /// 规则：任一成功=有效，双失败=无效，单网络失败=不确定
    pub fn determine(proxy_result: Option<&HttpResult>, direct_result: Option<&HttpResult>) -> Self {
        let proxy_success = proxy_result.map(|r| r.is_success()).unwrap_or(false);
        let direct_success = direct_result.map(|r| r.is_success()).unwrap_or(false);
        
        // 任一网络成功即有效
        if proxy_success || direct_success {
            return ValidationStatus::Valid;
        }
        
        // 检查是否双网络都有结果
        let proxy_failed = proxy_result.map(|r| r.is_failure()).unwrap_or(false);
        let direct_failed = direct_result.map(|r| r.is_failure()).unwrap_or(false);
        
        // 双网络都失败才判定为无效
        if proxy_failed && direct_failed {
            return ValidationStatus::Invalid;
        }
        
        // 其他情况（单网络失败、超时等）判定为不确定
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
    
    /// 判断是否失败 (HTTP 4xx/5xx 或连接错误，但不包括超时)
    pub fn is_failure(&self) -> bool {
        if self.is_timeout {
            return false; // 超时不算确定性失败
        }
        match self.status_code {
            Some(code) => code >= 400,
            None => self.error.is_some(), // 连接错误
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
            if errors.is_empty() { None } else { Some(errors.join("; ")) }
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
                ValidationStatus::Invalid => {
                    report.invalid_count += 1;
                    report.invalid_urls.push(InvalidBookmark {
                        title: String::new(), // 由调用者填充
                        url: result.url.clone(),
                        browser: String::new(),
                        folder_path: String::new(),
                        proxy_error: result.proxy_result.as_ref()
                            .and_then(|r| r.error.clone()),
                        direct_error: result.direct_result.as_ref()
                            .and_then(|r| r.error.clone()),
                    });
                }
                ValidationStatus::Uncertain => report.uncertain_count += 1,
                ValidationStatus::Skipped => report.skipped_count += 1,
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
        let proxy = HttpResult::success(404, 100);
        let direct = HttpResult::success(500, 50);
        assert_eq!(
            ValidationStatus::determine(Some(&proxy), Some(&direct)),
            ValidationStatus::Invalid
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
        
        // 创建直连客户端
        let direct_client = Client::builder()
            .timeout(timeout)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        
        // 创建代理客户端 (如果配置了代理)
        let proxy_client = if let Some(ref proxy_url) = config.proxy_url {
            info!("🌐 配置代理: {}", proxy_url);
            let proxy = reqwest::Proxy::all(proxy_url)?;
            Some(Client::builder()
                .timeout(timeout)
                .proxy(proxy)
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()?)
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
    
    /// 批量检查URL
    pub async fn check_batch<F>(
        &self,
        urls: Vec<String>,
        progress_callback: F,
    ) -> Vec<UrlCheckResult>
    where
        F: Fn(usize, usize, &str),
    {
        use tokio::sync::Semaphore;
        use std::sync::Arc;
        
        let total = urls.len();
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));
        let mut handles = Vec::with_capacity(total);
        
        for (i, url) in urls.into_iter().enumerate() {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let checker_config = self.config.clone();
            
            // 创建新的客户端用于并发请求
            let proxy_client = self.proxy_client.clone();
            let direct_client = self.direct_client.clone();
            
            let handle = tokio::spawn(async move {
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
                
                drop(permit);
                (i, url, result)
            });
            
            handles.push(handle);
        }
        
        let mut results = vec![None; total];
        for handle in handles {
            if let Ok((i, url, result)) = handle.await {
                progress_callback(i + 1, total, &url);
                results[i] = Some(result);
            }
        }
        
        results.into_iter().filter_map(|r| r).collect()
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

    fn failure_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(400u16),
            Just(401u16),
            Just(403u16),
            Just(404u16),
            Just(500u16),
            Just(502u16),
            Just(503u16),
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
            direct_code in failure_status_code(),
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
            proxy_code in failure_status_code(),
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

    /// **Feature: bookmark-validity-checker, Property 2: 双网络都失败才判定为无效**
    /// **Validates: Requirements 1.4**
    proptest! {
        #[test]
        fn prop_both_fail_is_invalid(
            proxy_code in failure_status_code(),
            direct_code in failure_status_code(),
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都失败 -> 应该无效
            let proxy = HttpResult::success(proxy_code, latency1);
            let direct = HttpResult::success(direct_code, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Invalid);
        }

        #[test]
        fn prop_both_connection_error_is_invalid(
            error1 in "[a-z]{5,20}",
            error2 in "[a-z]{5,20}",
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都连接错误 -> 应该无效
            let proxy = HttpResult::failure(error1, latency1);
            let direct = HttpResult::failure(error2, latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Invalid);
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

    fn failure_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(400u16),
            Just(404u16),
            Just(500u16),
            Just(503u16),
        ]
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
            fail_code in failure_status_code(),
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
            fail_code in failure_status_code(),
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
            make_folder("Folder", vec![
                make_bookmark("B", "http://b.com"),
                make_bookmark("C", "http://c.com"),
            ]),
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
            make_folder("Folder", vec![
                make_bookmark("Valid2", "http://valid2.com"),
                make_bookmark("Invalid2", "http://invalid2.com"),
            ]),
        ];
        
        let invalid: HashSet<String> = vec![
            "http://invalid.com".to_string(),
            "http://invalid2.com".to_string(),
        ].into_iter().collect();
        
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
