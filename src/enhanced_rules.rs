//! 增强分类规则引擎
//! 
//! 支持更精确的URL和标题匹配，包括：
//! - 域名通配符匹配
//! - 路径前缀和正则匹配
//! - 查询参数匹配
//! - 关键词包含/排除
//! - AND/OR组合逻辑

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// URL匹配条件
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UrlMatcher {
    /// 域名模式 (支持通配符 *.example.com)
    #[serde(default)]
    pub domain_patterns: Vec<String>,
    /// 路径前缀匹配 (如 /user/, /api/)
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    /// 路径正则表达式
    #[serde(default)]
    pub path_regex: Option<String>,
    /// 查询参数匹配 (参数名 -> 值模式)
    #[serde(default)]
    pub query_params: Option<HashMap<String, String>>,
    /// 完整URL正则表达式
    #[serde(default)]
    pub full_url_regex: Option<String>,
}

/// 标题匹配条件
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TitleMatcher {
    /// 必须包含的关键词 (任一匹配即可)
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 必须排除的关键词 (任一匹配则排除)
    #[serde(default)]
    pub excludes: Vec<String>,
    /// 标题正则表达式
    #[serde(default)]
    pub regex: Option<String>,
}

/// 条件组合逻辑
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub enum MatchLogic {
    #[default]
    And,  // URL和Title条件都必须满足
    Or,   // URL或Title任一满足即可
}

/// 增强的匹配条件
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatchCondition {
    #[serde(default)]
    pub url: Option<UrlMatcher>,
    #[serde(default)]
    pub title: Option<TitleMatcher>,
    #[serde(default)]
    pub logic: MatchLogic,
}

/// 匹配结果 (用于调试)
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub matched: bool,
    pub rule_name: Option<String>,
    pub match_reason: String,
    pub all_candidates: Vec<String>,
}

impl UrlMatcher {
    /// 检查URL是否匹配
    pub fn matches(&self, url: &str) -> bool {
        // 解析URL
        let url_lower = url.to_lowercase();
        
        // 提取域名和路径
        let (domain, path, query) = parse_url_parts(&url_lower);
        
        // 检查域名模式
        if !self.domain_patterns.is_empty() {
            let domain_matched = self.domain_patterns.iter().any(|pattern| {
                match_domain_pattern(&domain, pattern)
            });
            if !domain_matched {
                return false;
            }
        }
        
        // 检查路径前缀
        if !self.path_prefixes.is_empty() {
            let prefix_matched = self.path_prefixes.iter().any(|prefix| {
                path.starts_with(&prefix.to_lowercase())
            });
            if !prefix_matched {
                return false;
            }
        }
        
        // 检查路径正则
        if let Some(ref regex_str) = self.path_regex {
            if let Ok(re) = Regex::new(regex_str) {
                if !re.is_match(&path) {
                    return false;
                }
            }
        }
        
        // 检查查询参数
        if let Some(ref params) = self.query_params {
            let query_map = parse_query_params(&query);
            for (key, value_pattern) in params {
                match query_map.get(key) {
                    Some(actual_value) => {
                        if !actual_value.contains(&value_pattern.to_lowercase()) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }
        
        // 检查完整URL正则
        if let Some(ref regex_str) = self.full_url_regex {
            if let Ok(re) = Regex::new(regex_str) {
                if !re.is_match(url) {
                    return false;
                }
            }
        }
        
        true
    }
}

impl TitleMatcher {
    /// 检查标题是否匹配
    pub fn matches(&self, title: &str) -> bool {
        let title_lower = title.to_lowercase();
        
        // 检查排除词 (任一匹配则排除)
        if self.excludes.iter().any(|ex| title_lower.contains(&ex.to_lowercase())) {
            return false;
        }
        
        // 检查关键词 (任一匹配即可)
        if !self.keywords.is_empty() {
            let keyword_matched = self.keywords.iter().any(|kw| {
                title_lower.contains(&kw.to_lowercase())
            });
            if !keyword_matched {
                return false;
            }
        }
        
        // 检查正则
        if let Some(ref regex_str) = self.regex {
            if let Ok(re) = Regex::new(regex_str) {
                if !re.is_match(title) {
                    return false;
                }
            }
        }
        
        true
    }
}

impl MatchCondition {
    /// 检查是否匹配
    pub fn matches(&self, url: &str, title: &str) -> bool {
        let url_matched = self.url.as_ref().map(|m| m.matches(url)).unwrap_or(true);
        let title_matched = self.title.as_ref().map(|m| m.matches(title)).unwrap_or(true);
        
        match self.logic {
            MatchLogic::And => url_matched && title_matched,
            MatchLogic::Or => {
                // 如果两个条件都存在，任一满足即可
                if self.url.is_some() && self.title.is_some() {
                    url_matched || title_matched
                } else {
                    // 如果只有一个条件，必须满足
                    url_matched && title_matched
                }
            }
        }
    }
}

/// 解析URL的各部分
fn parse_url_parts(url: &str) -> (String, String, String) {
    let url = url.trim_start_matches("http://").trim_start_matches("https://");
    
    let (domain_path, query) = match url.find('?') {
        Some(pos) => (&url[..pos], &url[pos+1..]),
        None => (url, ""),
    };
    
    let (domain, path) = match domain_path.find('/') {
        Some(pos) => (&domain_path[..pos], &domain_path[pos..]),
        None => (domain_path, "/"),
    };
    
    (domain.to_string(), path.to_string(), query.to_string())
}

/// 匹配域名模式 (支持通配符)
fn match_domain_pattern(domain: &str, pattern: &str) -> bool {
    let pattern_lower = pattern.to_lowercase();
    
    if pattern_lower.starts_with("*.") {
        // 通配符匹配: *.example.com 匹配 sub.example.com 和 example.com
        let suffix = &pattern_lower[2..];
        domain.ends_with(suffix) || domain == suffix
    } else {
        // 精确匹配
        domain == pattern_lower
    }
}

/// 解析查询参数
fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for pair in query.split('&') {
        if let Some(pos) = pair.find('=') {
            let key = &pair[..pos];
            let value = &pair[pos+1..];
            params.insert(key.to_string(), value.to_string());
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_pattern_exact() {
        assert!(match_domain_pattern("example.com", "example.com"));
        assert!(!match_domain_pattern("example.com", "other.com"));
    }

    #[test]
    fn test_domain_pattern_wildcard() {
        assert!(match_domain_pattern("sub.example.com", "*.example.com"));
        assert!(match_domain_pattern("example.com", "*.example.com"));
        assert!(!match_domain_pattern("other.com", "*.example.com"));
    }

    #[test]
    fn test_url_matcher_domain() {
        let matcher = UrlMatcher {
            domain_patterns: vec!["*.github.com".to_string()],
            ..Default::default()
        };
        assert!(matcher.matches("https://github.com/user/repo"));
        assert!(matcher.matches("https://gist.github.com/user"));
        assert!(!matcher.matches("https://gitlab.com/user"));
    }

    #[test]
    fn test_url_matcher_path_prefix() {
        let matcher = UrlMatcher {
            domain_patterns: vec!["github.com".to_string()],
            path_prefixes: vec!["/user/".to_string()],
            ..Default::default()
        };
        assert!(matcher.matches("https://github.com/user/repo"));
        assert!(!matcher.matches("https://github.com/org/repo"));
    }

    #[test]
    fn test_title_matcher_keywords() {
        let matcher = TitleMatcher {
            keywords: vec!["github".to_string(), "gitlab".to_string()],
            ..Default::default()
        };
        assert!(matcher.matches("My GitHub Project"));
        assert!(matcher.matches("GitLab CI/CD"));
        assert!(!matcher.matches("My Project"));
    }

    #[test]
    fn test_title_matcher_excludes() {
        let matcher = TitleMatcher {
            keywords: vec!["project".to_string()],
            excludes: vec!["private".to_string()],
            ..Default::default()
        };
        assert!(matcher.matches("My Public Project"));
        assert!(!matcher.matches("My Private Project"));
    }

    #[test]
    fn test_match_condition_and() {
        let condition = MatchCondition {
            url: Some(UrlMatcher {
                domain_patterns: vec!["github.com".to_string()],
                ..Default::default()
            }),
            title: Some(TitleMatcher {
                keywords: vec!["repo".to_string()],
                ..Default::default()
            }),
            logic: MatchLogic::And,
        };
        assert!(condition.matches("https://github.com/user/repo", "My Repo"));
        assert!(!condition.matches("https://github.com/user/repo", "My Project"));
        assert!(!condition.matches("https://gitlab.com/user/repo", "My Repo"));
    }

    #[test]
    fn test_match_condition_or() {
        let condition = MatchCondition {
            url: Some(UrlMatcher {
                domain_patterns: vec!["github.com".to_string()],
                ..Default::default()
            }),
            title: Some(TitleMatcher {
                keywords: vec!["repo".to_string()],
                ..Default::default()
            }),
            logic: MatchLogic::Or,
        };
        assert!(condition.matches("https://github.com/user/repo", "My Repo"));
        assert!(condition.matches("https://github.com/user/repo", "My Project")); // URL matches
        assert!(condition.matches("https://gitlab.com/user/repo", "My Repo")); // Title matches
        assert!(!condition.matches("https://gitlab.com/user/repo", "My Project")); // Neither matches
    }
}


#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    /// **Feature: bookmark-validity-checker, Property 15: 路径前缀匹配正确性**
    /// **Validates: Requirements 8.2**
    proptest! {
        #[test]
        fn prop_path_prefix_match(
            prefix in "/[a-z]{1,5}/",
            suffix in "[a-z]{1,10}",
        ) {
            let full_path = format!("{}{}", prefix, suffix);
            let url = format!("https://example.com{}", full_path);
            
            let matcher = UrlMatcher {
                domain_patterns: vec!["example.com".to_string()],
                path_prefixes: vec![prefix.clone()],
                ..Default::default()
            };
            
            // 路径以前缀开头应该匹配
            prop_assert!(matcher.matches(&url));
        }

        #[test]
        fn prop_path_prefix_no_match(
            prefix in "/[a-z]{1,5}/",
            other_prefix in "/[0-9]{1,5}/",
        ) {
            let url = format!("https://example.com{}", other_prefix);
            
            let matcher = UrlMatcher {
                domain_patterns: vec!["example.com".to_string()],
                path_prefixes: vec![prefix],
                ..Default::default()
            };
            
            // 路径不以前缀开头不应该匹配
            prop_assert!(!matcher.matches(&url));
        }
    }

    /// **Feature: bookmark-validity-checker, Property 12: 关键词包含/排除逻辑**
    /// **Validates: Requirements 7.2**
    proptest! {
        #[test]
        fn prop_keyword_include(
            keyword in "[a-z]{3,8}",
            prefix in "[A-Z]{1,5} ",
            suffix in " [a-z]{1,5}",
        ) {
            let title = format!("{}{}{}", prefix, keyword, suffix);
            
            let matcher = TitleMatcher {
                keywords: vec![keyword.clone()],
                ..Default::default()
            };
            
            // 标题包含关键词应该匹配
            prop_assert!(matcher.matches(&title));
        }

        #[test]
        fn prop_keyword_exclude(
            keyword in "[a-z]{3,8}",
            exclude in "[a-z]{3,8}",
        ) {
            // 确保keyword和exclude不同
            prop_assume!(keyword != exclude);
            
            let title = format!("Title with {} and {}", keyword, exclude);
            
            let matcher = TitleMatcher {
                keywords: vec![keyword],
                excludes: vec![exclude],
                ..Default::default()
            };
            
            // 标题包含排除词不应该匹配
            prop_assert!(!matcher.matches(&title));
        }
    }

    /// **Feature: bookmark-validity-checker, Property 13: AND/OR组合逻辑**
    /// **Validates: Requirements 7.3**
    proptest! {
        #[test]
        fn prop_and_logic_both_true(
            domain in "[a-z]{3,8}\\.com",
            keyword in "[a-z]{3,8}",
        ) {
            let url = format!("https://{}/path", domain);
            let title = format!("Title with {}", keyword);
            
            let condition = MatchCondition {
                url: Some(UrlMatcher {
                    domain_patterns: vec![domain],
                    ..Default::default()
                }),
                title: Some(TitleMatcher {
                    keywords: vec![keyword],
                    ..Default::default()
                }),
                logic: MatchLogic::And,
            };
            
            // AND逻辑：两个都满足应该匹配
            prop_assert!(condition.matches(&url, &title));
        }

        #[test]
        fn prop_and_logic_one_false(
            domain in "[a-z]{3,8}\\.com",
            other_domain in "[0-9]{3,8}\\.com",
            keyword in "[a-z]{3,8}",
        ) {
            let url = format!("https://{}/path", other_domain);
            let title = format!("Title with {}", keyword);
            
            let condition = MatchCondition {
                url: Some(UrlMatcher {
                    domain_patterns: vec![domain],
                    ..Default::default()
                }),
                title: Some(TitleMatcher {
                    keywords: vec![keyword],
                    ..Default::default()
                }),
                logic: MatchLogic::And,
            };
            
            // AND逻辑：URL不匹配，整体不应该匹配
            prop_assert!(!condition.matches(&url, &title));
        }

        #[test]
        fn prop_or_logic_one_true(
            domain in "[a-z]{3,8}\\.com",
            other_domain in "[0-9]{3,8}\\.com",
            keyword in "[a-z]{3,8}",
        ) {
            let url = format!("https://{}/path", other_domain);
            let title = format!("Title with {}", keyword);
            
            let condition = MatchCondition {
                url: Some(UrlMatcher {
                    domain_patterns: vec![domain],
                    ..Default::default()
                }),
                title: Some(TitleMatcher {
                    keywords: vec![keyword],
                    ..Default::default()
                }),
                logic: MatchLogic::Or,
            };
            
            // OR逻辑：标题匹配，整体应该匹配
            prop_assert!(condition.matches(&url, &title));
        }
    }
}
