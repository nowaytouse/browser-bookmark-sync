//!
//! 通过代理和直连双网络验证收藏夹URL的有效性。
//! 判断逻辑：任一网络成功即有效，双网络都失败才判定为无效。
//! DNS 解析策略：中国域名 → 阿里云 DoH，国际域名 → Cloudflare 1.1.1.1 `DoH`
//!
//! 死链判断五维度：
//!   1. DNS无法解析（域名不存在）→ Dead
//!   2. 真实无法访问（ECONNREFUSED 服务器主动拒绝）→ Dead
//!   3. 明确错误码（404 Not Found / 410 Gone）→ Dead
//!   4. 本地网络/代理IO错误（超时/本地故障）→ Uncertain（本地原因，不判死链）
//!   5. 服务器暂时维护（5xx / 503）→ Uncertain（存疑保留）

use anyhow::Result;
use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig, ResolverOpts, CLOUDFLARE},
    net::runtime::TokioRuntimeProvider,
    TokioResolver,
};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tracing::{debug, info};

// ─────────────────────────────────────────────────────────────
// 智能 DoH DNS 路由解析器
// CN 域名 → 阿里云 DoH (dns.alidns.com / 223.5.5.5)
// 国际域名 → Cloudflare DoH (1.1.1.1 / 1.0.0.1)
// ─────────────────────────────────────────────────────────────

/// 中国大陆 TLD 与常用域名后缀
const CN_TLDS: &[&str] = &[
    ".cn", ".com.cn", ".net.cn", ".org.cn", ".gov.cn", ".edu.cn", ".mil.cn", ".ac.cn", ".ah.cn",
    ".bj.cn", ".cq.cn", ".fj.cn", ".gd.cn", ".gs.cn", ".gz.cn", ".gx.cn", ".ha.cn", ".hb.cn",
    ".he.cn", ".hi.cn", ".hk.cn", ".hl.cn", ".hn.cn", ".jl.cn", ".js.cn", ".jx.cn", ".ln.cn",
    ".mo.cn", ".nm.cn", ".nx.cn", ".qh.cn", ".sc.cn", ".sd.cn", ".sh.cn", ".sn.cn", ".sx.cn",
    ".tj.cn", ".tw.cn", ".xj.cn", ".xz.cn", ".yn.cn", ".zj.cn",
];

/// 中国大陆主要互联网服务商的主域名（二级域名归属）
const CN_APEX_DOMAINS: &[&str] = &[
    "baidu.com",
    "qq.com",
    "taobao.com",
    "tmall.com",
    "jd.com",
    "weibo.com",
    "sina.com",
    "163.com",
    "126.com",
    "sohu.com",
    "youku.com",
    "iqiyi.com",
    "bilibili.com",
    "zhihu.com",
    "douban.com",
    "toutiao.com",
    "bytedance.com",
    "aliyun.com",
    "alibaba.com",
    "aliexpress.com",
    "alipay.com",
    "wechat.com",
    "weixin.qq.com",
    "csdn.net",
    "cnblogs.com",
    "jianshu.com",
    "douyin.com",
    "kuaishou.com",
    "pinduoduo.com",
    "meituan.com",
    "dianping.com",
    "58.com",
    "anjuke.com",
    "ganji.com",
    "360.cn",
    "qihu.com",
    "sogou.com",
    "so.com",
    "soso.com",
    "huawei.com",
    "xiaomi.com",
    "oppo.com",
    "vivo.com",
    "lenovo.com",
    "haier.com",
    "mi.com",
    "netease.com",
    "tencent.com",
    "ifeng.com",
    "xinhuanet.com",
    "people.com.cn",
    "cctv.com",
    "chinadaily.com.cn",
    "pku.edu.cn",
    "tsinghua.edu.cn",
    "gov.cn",
];

/// 判断 hostname 是否属于中国大陆域名
fn is_cn_domain(host: &str) -> bool {
    let host = host.to_lowercase();
    // 检查 CN TLD
    for tld in CN_TLDS {
        if host.ends_with(tld) || host == tld.trim_start_matches('.') {
            return true;
        }
    }
    // 检查中国大陆主流顶级域名
    for apex in CN_APEX_DOMAINS {
        if host == *apex || host.ends_with(&format!(".{apex}")) {
            return true;
        }
    }
    false
}

/// 智能 `DoH` DNS 解析器
/// - CN 域名 → 阿里云 `DoH` (dns.alidns.com / 223.5.5.5)
/// - 国际域名 → Cloudflare `DoH` (1.1.1.1 / 1.0.0.1)
#[derive(Clone)]
struct SmartDohResolver {
    cn_resolver: Arc<TokioResolver>,
    global_resolver: Arc<TokioResolver>,
}

impl SmartDohResolver {
    fn build() -> Result<Self> {
        // 阿里云 DoH: https://dns.alidns.com/dns-query
        let cn_resolver = {
            let ali_ns1 = NameServerConfig::https(
                "223.5.5.5".parse::<IpAddr>()?,
                Arc::from("dns.alidns.com"),
                None,
            );
            let ali_ns2 = NameServerConfig::https(
                "223.6.6.6".parse::<IpAddr>()?,
                Arc::from("dns.alidns.com"),
                None,
            );
            let cfg = ResolverConfig::from_parts(None, vec![], vec![ali_ns1, ali_ns2]);
            let mut opts = ResolverOpts::default();
            opts.timeout = Duration::from_secs(5);
            opts.attempts = 2;
            Arc::new(
                TokioResolver::builder_with_config(cfg, TokioRuntimeProvider::default())
                    .with_options(opts)
                    .build()?,
            )
        };

        // Cloudflare DoH: https://1.1.1.1/dns-query (使用内置 CLOUDFLARE 常量)
        let global_resolver = {
            let mut opts = ResolverOpts::default();
            opts.timeout = Duration::from_secs(5);
            opts.attempts = 2;
            Arc::new(
                TokioResolver::builder_with_config(
                    ResolverConfig::https(&CLOUDFLARE),
                    TokioRuntimeProvider::default(),
                )
                .with_options(opts)
                .build()?,
            )
        };

        Ok(Self {
            cn_resolver,
            global_resolver,
        })
    }
}

impl Resolve for SmartDohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().trim_end_matches('.').to_string();
        let cn_res = Arc::clone(&self.cn_resolver);
        let global_res = Arc::clone(&self.global_resolver);
        Box::pin(async move {
            let resolver = if is_cn_domain(&host) {
                debug!("[DNS] CN 域名 → 阿里云 DoH: {}", host);
                cn_res
            } else {
                debug!("[DNS] 国际域名 → Cloudflare DoH: {}", host);
                global_res
            };
            let lookup = resolver
                .lookup_ip(host.as_str())
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            let addrs: Addrs = Box::new(
                lookup
                    .iter()
                    .map(|ip: IpAddr| SocketAddr::new(ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

/// URL检查结果状态
///
/// 五维度死链判断分类：
///   维度1 DNS无法解析          → `DnsResolutionFailed`  → 确认死链
///   维度2 服务器主动拒绝连接    → `ConnectionRefused`    → 确认死链
///   维度3 明确错误码(404/410)  → `NotFound` / Gone      → 确认死链
///   维度4 本地网络/代理IO错误   → `LocalNetworkError`    → 存疑保留（本地原因）
///   维度5 服务器暂时维护/拦截   → `ServerError等`        → 存疑保留
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    /// 有效 - HTTP 2xx
    Valid,
    /// 重定向 - HTTP 3xx（跟随重定向后成功算 Valid）
    Redirected,
    /// 【确认死链-维度1】DNS 无法解析域名 - 域名不存在或已被注销
    DnsResolutionFailed,
    /// 【确认死链-维度2】服务器主动拒绝连接 (ECONNREFUSED) - 端口未监听或服务已停止
    ConnectionRefused,
    /// 【确认死链-维度3】资源明确不存在 - HTTP 404 Not Found
    NotFound,
    /// 【确认死链-维度3】资源已永久删除 - HTTP 410 Gone
    Gone,
    /// 【存疑保留-维度4】本地网络/代理 IO 错误 - 超时/本地故障，非服务器问题
    LocalNetworkError,
    /// 【存疑保留-维度4】请求超时 - 可能是网络慢或服务器慢，非确认死链
    Timeout,
    /// 【存疑保留-维度5】服务器暂时维护 - HTTP 502/503/504
    ServiceUnavailable,
    /// 【存疑保留-维度5】服务器内部错误 - HTTP 500/5xx
    ServerError,
    /// 【存疑保留】防火墙/Bot拦截 - HTTP 403 / Cloudflare WAF
    ForbiddenOrBotBlocked,
    /// 【存疑保留】触发限流 - HTTP 429
    RateLimited,
    /// 【存疑保留】需身份验证 - HTTP 401
    AuthenticationRequired,
    /// 【存疑保留】TLS/SSL握手错误 - 证书问题或协议不兼容
    TlsError,
    /// 不支持的协议 Scheme
    UnsupportedScheme,
    /// 不确定 - 其他无法分类的情况
    Uncertain,
    /// 简单无效（保留兼容旧代码）
    Invalid,
    /// 跳过 - 本地文件、javascript:等非HTTP URL
    Skipped,
}

impl ValidationStatus {
    /// 确认死链判断：
    ///   - `DnsResolutionFailed`: DNS 无法解析，域名不存在 (维度1)
    ///   - `ConnectionRefused`:   服务器主动拒绝，服务已停止 (维度2)
    ///   - `NotFound`:            HTTP 404，资源不存在 (维度3)
    ///   - Gone:                HTTP 410，资源已永久删除 (维度3)
    ///   - Invalid:             双路确认失败（兼容旧逻辑）
    ///
    /// 以下不会删除：
    ///   - Timeout / `LocalNetworkError`: 本地网络问题，不是服务器问题 (维度4)
    ///   - `ServiceUnavailable` / `ServerError`: 服务器临时故障 (维度5)
    ///   - `ForbiddenOrBotBlocked` / `RateLimited`: WAF/限流拦截，网站可能正常
    pub const fn should_auto_delete(&self) -> bool {
        matches!(
            self,
            Self::DnsResolutionFailed
                | Self::ConnectionRefused
                | Self::NotFound
                | Self::Gone
                | Self::Invalid
        )
    }

    /// 根据 HTTP 状态码分类
    pub const fn classify_http_code(code: u16) -> Self {
        match code {
            200..=299 => Self::Valid,
            300..=399 => Self::Redirected,
            401 => Self::AuthenticationRequired,
            403 => Self::ForbiddenOrBotBlocked,
            404 => Self::NotFound,
            410 => Self::Gone,
            429 => Self::RateLimited,
            502..=504 => Self::ServiceUnavailable,
            500..=599 => Self::ServerError,
            _ => Self::Uncertain,
        }
    }

    /// 根据代理和直连结果判定最终状态（五维度）
    pub fn determine(
        proxy_result: Option<&HttpResult>,
        direct_result: Option<&HttpResult>,
    ) -> Self {
        // ── 维度4：本地网络/代理错误 ── 任意一方为本地错误则存疑保留（不判死链）
        let proxy_local = proxy_result.is_some_and(|r| r.is_local_network_error);
        let direct_local = direct_result.is_some_and(|r| r.is_local_network_error);
        if proxy_local || direct_local {
            return Self::LocalNetworkError;
        }

        // ── 成功判断 ── 任一路成功即有效
        let proxy_success = proxy_result.is_some_and(HttpResult::is_success);
        let direct_success = direct_result.is_some_and(HttpResult::is_success);
        if proxy_success || direct_success {
            return Self::Valid;
        }

        // ── 维度5：WAF/限流/维护拦截 ── 存疑保留
        let proxy_blocked = proxy_result.is_some_and(HttpResult::is_blocked);
        let direct_blocked = direct_result.is_some_and(HttpResult::is_blocked);
        if proxy_blocked || direct_blocked {
            return Self::Uncertain;
        }

        // ── 必须双方都有结果才可能确定死链（单网络失败一律视为 Uncertain）──
        let (Some(proxy), Some(direct)) = (proxy_result, direct_result) else {
            return Self::Uncertain;
        };

        // ── 维度1/2/3：任何确认死链类型双路验证失败即认为死链 (Invalid) ──
        if proxy.is_single_attempt_strong_failure() && direct.is_single_attempt_strong_failure() {
            return Self::Invalid;
        }

        Self::Uncertain
    }
}

/// HTTP请求结果（含五维度错误分类标志）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResult {
    /// HTTP状态码 (None表示连接失败)
    pub status_code: Option<u16>,
    /// 错误信息
    pub error: Option<String>,
    /// 响应延迟(毫秒)
    pub latency_ms: u64,
    /// [维度4] 是否超时（本地原因，不判死链）
    pub is_timeout: bool,
    /// [维度1] DNS 解析失败（域名不存在）→ 确认死链
    pub is_dns_error: bool,
    /// [维度2] 服务器主动拒绝连接 ECONNREFUSED → 确认死链
    pub is_connection_refused: bool,
    /// [维度4] 本地网络/代理 IO 错误（非服务器原因）→ 存疑保留
    pub is_local_network_error: bool,
}

impl HttpResult {
    /// 创建成功结果
    pub const fn success(status_code: u16, latency_ms: u64) -> Self {
        Self {
            status_code: Some(status_code),
            error: None,
            latency_ms,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        }
    }

    /// 创建超时结果 [维度4]
    pub fn timeout(latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some("Request timeout".to_string()),
            latency_ms,
            is_timeout: true,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        }
    }

    /// 创建 DNS 解析失败结果 [维度1]
    pub const fn dns_error(error: String, latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some(error),
            latency_ms,
            is_timeout: false,
            is_dns_error: true,
            is_connection_refused: false,
            is_local_network_error: false,
        }
    }

    /// 创建连接拒绝结果 [维度2]
    pub const fn connection_refused(error: String, latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some(error),
            latency_ms,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: true,
            is_local_network_error: false,
        }
    }

    /// 创建本地网络/代理 IO 错误结果 [维度4]
    pub const fn local_network_error(error: String, latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some(error),
            latency_ms,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: true,
        }
    }

    /// 创建通用失败结果（未分类）
    pub const fn failure(error: String, latency_ms: u64) -> Self {
        Self {
            status_code: None,
            error: Some(error),
            latency_ms,
            is_timeout: false,
            is_dns_error: false,
            is_connection_refused: false,
            is_local_network_error: false,
        }
    }

    /// 判断是否成功 (HTTP 2xx/3xx)
    pub fn is_success(&self) -> bool {
        match self.status_code {
            Some(code) => (200..400).contains(&code),
            None => false,
        }
    }

    /// 判断是否是单轮明确资源不存在失败 [维度3]
    /// 仅 404/410 等明确资源不存在的状态码才算确定性失败
    /// 注意：DNS错误字符串、连接拒绝、451等一律为临时故障或存疑保留
    pub const fn is_single_attempt_strong_failure(&self) -> bool {
        if self.is_timeout
            || self.is_local_network_error
            || self.is_dns_error
            || self.is_connection_refused
        {
            return false;
        }
        match self.status_code {
            Some(code) => matches!(code, 404 | 410),
            None => false,
        }
    }

    /// 保留兼容旧名称（旧 `is_confirmed_dead` 现由单轮检测 `is_single_attempt_strong_failure` 驱动）
    pub fn is_confirmed_dead(&self) -> bool {
        self.is_single_attempt_strong_failure()
    }

    /// 保留兼容旧 `is_failure` 接口
    pub fn is_failure(&self) -> bool {
        self.is_single_attempt_strong_failure()
    }

    /// 判断是否是"存疑保留"状态码 [维度5]
    /// WAF/限流/CF拦截/服务器维护等，浏览器通常可以正常访问
    pub const fn is_blocked(&self) -> bool {
        match self.status_code {
            Some(code) => {
                // 401 需要认证
                // 403 CF验证/WAF/防火墙
                // 429 限流
                // 451 法律方面原因受限，不得直接视为死链，应保护或审核
                // 500 服务器内部错误
                // 502/503/504 服务暂时不可用/维护
                // 520-530 Cloudflare 特定错误
                matches!(
                    code,
                    401 | 403 | 429 | 451 | 500 | 502 | 503 | 504 | 520..=530
                )
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
                    errors.push(format!("Proxy: {e}"));
                } else if let Some(code) = pr.status_code {
                    errors.push(format!("Proxy: HTTP {code}"));
                }
            }
            if let Some(ref dr) = direct_result {
                if let Some(ref e) = dr.error {
                    errors.push(format!("Direct: {e}"));
                } else if let Some(code) = dr.status_code {
                    errors.push(format!("Direct: HTTP {code}"));
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
    /// 代理服务器URL (如 <http://127.0.0.1:7890>)
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
    pub const fn is_consistent(&self) -> bool {
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
    fn popular_domains_and_lookalikes_are_not_skipped() {
        assert_eq!(
            UrlChecker::should_skip("https://github.com/org/deleted"),
            None
        );
        assert_eq!(
            UrlChecker::should_skip("https://github.com.evil.example/post"),
            None
        );
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
        let connect_timeout = Duration::from_secs(10);

        // 构建智能 DoH DNS 解析器（CN→阿里云, 国际→Cloudflare 1.1.1.1）
        let doh_resolver = SmartDohResolver::build().ok();
        if doh_resolver.is_some() {
            info!("🌐 DNS: 智能 DoH 路由已启用 (CN→阿里云 223.5.5.5 | 国际→Cloudflare 1.1.1.1)");
        } else {
            info!("⚠️  DNS: DoH 初始化失败，回退至系统 DNS");
        }

        // 创建直连客户端（带 DoH 解析器）
        let direct_client = {
            let mut builder = Client::builder()
                .timeout(timeout)
                .connect_timeout(connect_timeout)
                .pool_idle_timeout(Duration::from_secs(30))
                .pool_max_idle_per_host(10)
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                .redirect(reqwest::redirect::Policy::limited(5));
            if let Some(ref r) = doh_resolver {
                builder = builder.dns_resolver(Arc::new(r.clone()));
            }
            builder.build()?
        };

        // 创建代理客户端（如果配置了代理，代理自带 DNS，不注入 DoH）
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
            debug!("未配置代理，仅使用直连模式（DoH 已接管 DNS）");
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

    /// 执行单个HTTP请求（带五维度错误分类）
    /// 先尝试 HEAD，如果返回 405 则回退到 GET
    async fn do_request(client: &Client, url: &str) -> HttpResult {
        let start = std::time::Instant::now();

        match client.head(url).send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                let latency = start.elapsed().as_millis() as u64;
                if status == 405 {
                    debug!("HEAD 返回 405，回退到 GET: {}", url);
                    return Self::do_get_request(client, url, start).await;
                }
                HttpResult::success(status, latency)
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                Self::classify_request_error(e, latency)
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
                Self::classify_request_error(e, latency)
            }
        }
    }

    /// 将 reqwest 错误分类为五维度标志位
    fn classify_request_error(e: reqwest::Error, latency_ms: u64) -> HttpResult {
        let err_str = e.to_string();
        let err_lower = err_str.to_lowercase();

        // [维度4] 超时 → 本地原因，存疑保留
        if e.is_timeout() {
            return HttpResult::timeout(latency_ms);
        }

        // [维度1] DNS 解析失败 → 域名不存在，确认死链
        if err_lower.contains("dns")
            || err_lower.contains("no such host")
            || err_lower.contains("name resolution")
            || err_lower.contains("name or service not known")
            || err_lower.contains("getaddrinfo")
            || err_lower.contains("nodename nor servname")
            || err_lower.contains("failed to lookup")
            || err_lower.contains("could not resolve host")
        {
            return HttpResult::dns_error(err_str, latency_ms);
        }

        // [维度2] 服务器主动拒绝连接 → 服务已停止，确认死链
        if err_lower.contains("connection refused")
            || err_lower.contains("econnrefused")
            || err_lower.contains("actively refused")
        {
            return HttpResult::connection_refused(err_str, latency_ms);
        }

        // [维度4] 本地网络/代理 IO 错误 → 存疑保留（非服务器问题）
        // 包括：连接重置、代理错误、TLS握手失败、本地IO错误等
        if e.is_connect()
            || e.is_body()
            || e.is_request()
            || err_lower.contains("connection reset")
            || err_lower.contains("econnreset")
            || err_lower.contains("broken pipe")
            || err_lower.contains("proxy")
            || err_lower.contains("tls")
            || err_lower.contains("ssl")
            || err_lower.contains("certificate")
            || err_lower.contains("handshake")
        {
            return HttpResult::local_network_error(err_str, latency_ms);
        }

        // 其他未分类错误
        HttpResult::failure(err_str, latency_ms)
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

    /// 提取 URL 的主域名根地址 (e.g. "<https://www.youtube.com/watch?v=1>" -> "<https://www.youtube.com>")
    /// 仅验证主层级域名，极大减少网络负载、防止被限流与 WAF 封禁，且免除具体页面内容变更导致的 404 误判
    pub fn extract_domain_origin(url_str: &str) -> String {
        if let Ok(parsed) = reqwest::Url::parse(url_str) {
            if let Some(host) = parsed.host_str() {
                let scheme = parsed.scheme();
                let port_str = match parsed.port() {
                    Some(p) => format!(":{p}"),
                    None => String::new(),
                };
                return format!("{scheme}://{host}{port_str}");
            }
        }
        url_str.to_string()
    }

    /// 批量检查URL - 采用【主层级域名去重校验】与【慢速防限流模式】，极度轻量且准确
    pub async fn check_batch<F>(
        &self,
        urls: Vec<String>,
        progress_callback: F,
    ) -> Vec<UrlCheckResult>
    where
        F: Fn(usize, usize, &str),
    {
        use futures::stream::{self, StreamExt};
        use std::collections::HashMap;

        let total_urls = urls.len();
        let concurrency = self.config.concurrency;

        // 1. 按照“最主层次域名”去重汇总（例如所有 youtube.com 视频只测 https://www.youtube.com 一次）
        let mut origin_map: HashMap<String, Vec<String>> = HashMap::new();
        for url in &urls {
            let origin = Self::extract_domain_origin(url);
            origin_map.entry(origin).or_default().push(url.clone());
        }
        let unique_origins: Vec<String> = origin_map.keys().cloned().collect();
        let total_origins = unique_origins.len();
        let reduction_pct = if total_urls > 0 {
            (total_urls.saturating_sub(total_origins)) * 100 / total_urls
        } else {
            0
        };

        let mut skipped_origins_count = 0;
        for origin in &unique_origins {
            if Self::should_skip(origin).is_some() {
                skipped_origins_count += 1;
            }
        }
        let to_check_count = total_origins.saturating_sub(skipped_origins_count);

        info!(
            "🌐 主层级域名去重校验启用: 原始 {} 条书签 -> {} 个独立主层级域名 (缩减 {}%, 其中 {} 个大站白名单豁免，实际仅网络检测 {} 个域名)",
            total_urls, total_origins, reduction_pct, skipped_origins_count, to_check_count
        );
        info!(
            "🐢 慢速防限流模式: 并发数 {}, 每个主域名附带 300ms~1000ms 防爬抖动延迟",
            concurrency
        );

        // 2. 仅对独立的主层级域名进行网络与 DNS 校验
        let origin_results_vec: Vec<(String, UrlCheckResult)> =
            stream::iter(unique_origins.into_iter().enumerate())
                .map(|(i, origin)| {
                    let proxy_client = self.proxy_client.clone();
                    let direct_client = self.direct_client.clone();

                    async move {
                        let result = if let Some(reason) = Self::should_skip(&origin) {
                            UrlCheckResult::skipped(origin.clone(), reason)
                        } else {
                            let jitter_ms = 300 + ((i as u64 * 137) % 700);
                            tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

                            let (proxy_result, direct_result) = if let Some(ref pc) = proxy_client {
                                let proxy_fut = Self::do_request(pc, &origin);
                                let direct_fut = Self::do_request(&direct_client, &origin);
                                let (pr, dr) = tokio::join!(proxy_fut, direct_fut);
                                (Some(pr), Some(dr))
                            } else {
                                let dr = Self::do_request(&direct_client, &origin).await;
                                (None, Some(dr))
                            };
                            UrlCheckResult::from_results(
                                origin.clone(),
                                proxy_result,
                                direct_result,
                            )
                        };
                        (i, origin, result)
                    }
                })
                .buffer_unordered(concurrency)
                .inspect(|(i, origin, _)| {
                    progress_callback(*i + 1, total_origins, origin);
                })
                .map(|(_, origin, result)| (origin, result))
                .collect()
                .await;

        let origin_results: HashMap<String, UrlCheckResult> =
            origin_results_vec.into_iter().collect();

        // 3. 将主域名的有效性结论完整映射回每一条原始书签 URL
        urls.into_iter()
            .map(|url| {
                let origin = Self::extract_domain_origin(&url);
                if let Some(res) = origin_results.get(&origin) {
                    let mut mapped = res.clone();
                    mapped.url = url;
                    mapped
                } else {
                    UrlCheckResult::skipped(url, "未找到主域名检测结论")
                }
            })
            .collect()
    }

    /// 深度网页内容有效性检测 (Content Validity Check)
    /// 本功能为独立检测：前提是该网站主域名有效。
    /// 专门检测全路径 URL（例如具体视频或具体博文），当主站可达但具体网页返回 404/410 时，判定为失效内容并果断清除；
    /// 反爬、限流、5xx 或网络抖动等情况继续列入‘存疑保留’清单，绝不误删。
    pub async fn check_batch_content<F>(
        &self,
        urls: Vec<String>,
        progress_callback: F,
    ) -> Vec<UrlCheckResult>
    where
        F: Fn(usize, usize, &str),
    {
        use futures::stream::{self, StreamExt};

        let total_urls = urls.len();
        let concurrency = self.config.concurrency;

        info!(
            "🎯 深度内容有效性检测启用: 共 {} 条书签将逐项检查完整路径 (前提：网站主域名有效；仅对返回真实 404 Not Found / 410 Gone 的内容判定失效清理)",
            total_urls
        );
        info!(
            "🐢 慢速防限流模式: 并发数 {}, 每个请求附带 300ms~1000ms 防爬抖动延迟",
            concurrency
        );

        let results: Vec<UrlCheckResult> = stream::iter(urls.into_iter().enumerate())
            .map(|(i, url)| {
                let proxy_client = self.proxy_client.clone();
                let direct_client = self.direct_client.clone();

                async move {
                    let result = if let Some(reason) = Self::should_skip(&url) {
                        UrlCheckResult::skipped(url.clone(), reason)
                    } else {
                        let jitter_ms = 300 + ((i as u64 * 137) % 700);
                        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

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
                progress_callback(*i + 1, total_urls, url);
            })
            .map(|(_, _, result)| result)
            .collect()
            .await;

        results
    }

    /// 获取配置
    pub const fn config(&self) -> &CheckerConfig {
        &self.config
    }

    /// 是否配置了代理
    pub const fn has_proxy(&self) -> bool {
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

    // 确定性失败状态码（资源真的不存在，451为受限保护不可计入）
    fn definite_failure_status_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(404u16), // Not Found
            Just(410u16), // Gone
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

    // **Feature: bookmark-validity-checker, Property 1: 任一网络成功即判定为有效**
    // **Validates: Requirements 1.3**
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

    // **Feature: bookmark-validity-checker, Property 2: 双网络都返回404/410才判定为无效**
    // **Validates: Requirements 1.4**
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
        fn prop_dns_error_is_uncertain(
            latency1 in any_latency(),
            latency2 in any_latency(),
        ) {
            // 双网络都 DNS 解析失败（字符串形式） -> 应该为不确定（不可判定为确定死链）
            let proxy = HttpResult::failure("dns resolution failed".to_string(), latency1);
            let direct = HttpResult::failure("no such host".to_string(), latency2);
            let status = ValidationStatus::determine(Some(&proxy), Some(&direct));
            prop_assert_eq!(status, ValidationStatus::Uncertain);
        }
    }

    // **Feature: bookmark-validity-checker, Property 9: CF/WAF 拦截标记为不确定**
    // **Validates: Requirements 1.6 (新增)**
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

    // **Feature: bookmark-validity-checker, Property 8: 报告数量一致性**
    // **Validates: Requirements 4.2**
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

    // **Feature: bookmark-validity-checker, Property 7: 超时标记为不确定**
    // **Validates: Requirements 5.2**
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

    // **Feature: bookmark-validity-checker, Property 3: 单网络模式失败判定为不确定**
    // **Validates: Requirements 1.5**
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
    pub const fn dry_run(urls_to_delete: Vec<String>, preserved_count: usize) -> Self {
        Self {
            deleted_count: urls_to_delete.len(),
            preserved_count,
            backup_path: None,
            is_dry_run: true,
            urls_to_delete,
        }
    }

    /// 创建实际删除结果
    pub const fn actual(deleted_count: usize, preserved_count: usize, backup_path: String) -> Self {
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

pub fn extract_invalid_recursive(
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

    // **Feature: bookmark-validity-checker, Property 5: 仅删除高置信度无效项**
    // **Validates: Requirements 3.1**
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
                let url = format!("http://valid{i}.com");
                bookmarks.push(make_bookmark(&format!("Valid{i}"), &url));
                results.push(UrlCheckResult::from_results(
                    url,
                    Some(HttpResult::success(200, 100)),
                    Some(HttpResult::success(200, 50)),
                ));
            }

            // 生成无效收藏夹
            for i in 0..invalid_count {
                let url = format!("http://invalid{i}.com");
                bookmarks.push(make_bookmark(&format!("Invalid{i}"), &url));
                results.push(UrlCheckResult::from_results(
                    url.clone(),
                    Some(HttpResult::success(404, 100)),
                    Some(HttpResult::success(404, 50)),
                ));
                invalid_urls.insert(url);
            }

            // 生成不确定收藏夹
            for i in 0..uncertain_count {
                let url = format!("http://uncertain{i}.com");
                bookmarks.push(make_bookmark(&format!("Uncertain{i}"), &url));
                results.push(UrlCheckResult::from_results(
                    url,
                    None,
                    Some(HttpResult::success(404, 50)),
                ));
            }

            let _original_count = bookmarks.len();
            let removed = remove_invalid_bookmarks(&mut bookmarks, &invalid_urls);

            // 验证：只删除了Invalid状态的收藏夹
            prop_assert_eq!(removed, invalid_count);
            // 验证：Valid和Uncertain都被保留
            prop_assert_eq!(bookmarks.len(), valid_count + uncertain_count);
            // 验证：删除目标都是Invalid状态
            prop_assert!(validate_delete_targets(&results, &invalid_urls));
        }
    }

    // **Feature: bookmark-validity-checker, Property 6: Dry-run模式不修改数据**
    // **Validates: Requirements 3.4**
    proptest! {
        #[test]
        fn prop_dry_run_no_modification(
            bookmark_count in 1usize..20,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for i in 0..bookmark_count {
                let url = format!("http://test{i}.com");
                bookmarks.push(make_bookmark(&format!("Test{i}"), &url));
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
#[derive(Debug, Clone, Default)]
pub struct RemoveConfig {
    /// 是否保留空文件夹（默认 false = 删除空文件夹）
    pub keep_empty_folders: bool,
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
    if config.keep_empty_folders {
        // 统计保留的文件夹数量
        count_folders_recursive(bookmarks, &mut stats.folders_preserved);
    } else {
        cleanup_empty_folders_recursive(bookmarks, &mut stats);
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

    // **Feature: folder-structure-preservation, Property 1: 文件夹层级关系保持**
    // **Validates: Requirements 1.1, 1.3**
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
                let url = format!("http://valid{i}.com");
                bookmarks_in_folder.push(make_bookmark(&format!("Valid{i}"), &url));
                all_urls.push(url);
            }

            for i in 0..invalid_count {
                let url = format!("http://invalid{i}.com");
                bookmarks_in_folder.push(make_bookmark(&format!("Invalid{i}"), &url));
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

    // **Feature: folder-structure-preservation, Property 5: 只删除死链书签**
    // **Validates: Requirements 1.1**
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
                let url = format!("http://valid{i}.com");
                bookmarks.push(make_bookmark(&format!("Valid{i}"), &url));
                valid_urls.insert(url);
            }

            for i in 0..invalid_count {
                let url = format!("http://invalid{i}.com");
                bookmarks.push(make_bookmark(&format!("Invalid{i}"), &url));
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

    // **Feature: folder-structure-preservation, Property 2: 默认模式删除空文件夹**
    // **Validates: Requirements 2.1**
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
                    let url = format!("http://folder{f}_bookmark{b}.com");
                    children.push(make_bookmark(&format!("B{f}_{b}"), &url));
                    // 标记所有书签为死链
                    invalid_urls.insert(url);
                }
                bookmarks.push(make_folder(&format!("Folder{f}"), children));
            }

            let config = RemoveConfig::default(); // keep_empty_folders = false
            remove_invalid_bookmarks_preserve_structure(&mut bookmarks, &invalid_urls, &config);

            // 验证没有空文件夹
            prop_assert!(!has_empty_folders(&bookmarks));
        }
    }

    // **Feature: folder-structure-preservation, Property 3: 保留空文件夹模式**
    // **Validates: Requirements 2.2**
    proptest! {
        #[test]
        fn prop_keep_empty_folders(
            folder_count in 1usize..3,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for f in 0..folder_count {
                let url = format!("http://dead{f}.com");
                let children = vec![make_bookmark(&format!("Dead{f}"), &url)];
                bookmarks.push(make_folder(&format!("Folder{f}"), children));
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

    // **Feature: folder-structure-preservation, Property 4: 报告数量一致性**
    // **Validates: Requirements 1.4**
    proptest! {
        #[test]
        fn prop_report_consistency(
            valid_count in 1usize..10,
            invalid_count in 0usize..5,
        ) {
            let mut bookmarks = Vec::new();
            let mut invalid_urls = HashSet::new();

            for i in 0..valid_count {
                let url = format!("http://valid{i}.com");
                bookmarks.push(make_bookmark(&format!("Valid{i}"), &url));
            }

            for i in 0..invalid_count {
                let url = format!("http://invalid{i}.com");
                bookmarks.push(make_bookmark(&format!("Invalid{i}"), &url));
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

    // **Feature: folder-structure-preservation, Property 6: 导出保持结构**
    // **Validates: Requirements 6.1**
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
                let url = format!("http://test{i}.com");
                children.push(make_bookmark(&format!("Test{i}"), &url));
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

    // **Feature: folder-structure-preservation, Property 8: 路径可达性**
    // **Validates: Requirements 5.1**
    proptest! {
        #[test]
        fn prop_all_extracted_reachable(
            bookmark_count in 1usize..5,
        ) {
            let mut children = Vec::new();
            let mut target_urls = HashSet::new();

            for i in 0..bookmark_count {
                let url = format!("http://test{i}.com");
                children.push(make_bookmark(&format!("Test{i}"), &url));
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
