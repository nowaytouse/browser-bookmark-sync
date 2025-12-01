//! 书签清理模块 - 检测异常书签数据（仅检测，不自动删除）
//! 
//! 功能：
//! 1. 检测重复书签
//! 2. 检测NSFW内容（仅统计分类）
//! 3. 检测空文件夹
//!
//! ⚠️ 注意：本模块仅提供检测功能，不自动删除任何书签

use std::collections::HashMap;

use crate::browsers::Bookmark;

/// 异常检测结果
#[derive(Debug, Default)]
pub struct AnomalyReport {
    /// 重复URL数量
    pub duplicate_count: usize,
    /// NSFW内容数量（仅统计）
    pub nsfw_count: usize,
    /// 空文件夹数量
    pub empty_folder_count: usize,
}

/// NSFW域名模式
const NSFW_DOMAIN_PATTERNS: &[&str] = &[
    "pornhub.com", "xvideos.com", "xnxx.com", "xhamster.com",
    "redtube.com", "youporn.com", "tube8.com", "spankbang.com",
    "hanime.tv", "nhentai.net", "e-hentai.org", "exhentai.org",
    "rule34.xxx", "gelbooru.com", "danbooru.donmai.us",
    "iwara.tv", "kemono.party", "hitomi.la",
    "javlibrary.com", "javdb.com", "missav.com",
    "onlyfans.com", "fansly.com", "f95zone.to",
];

/// NSFW标题关键词
const NSFW_TITLE_KEYWORDS: &[&str] = &[
    "porn", "hentai", "nsfw", "adult", "xxx", "18+", "r18",
    "エロ", "成人", "工口", "同人誌",
];

impl AnomalyReport {
    pub fn print_summary(&self) {
        println!("\n📊 书签分析报告");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        if self.duplicate_count > 0 {
            println!("🔄 重复URL: {} 个", self.duplicate_count);
        }
        
        if self.empty_folder_count > 0 {
            println!("📁 空文件夹: {} 个", self.empty_folder_count);
        }
        
        // NSFW仅统计，不是问题
        if self.nsfw_count > 0 {
            println!("🔞 NSFW内容: {} 个", self.nsfw_count);
        }
        
        let total_issues = self.duplicate_count + self.empty_folder_count;
        
        if total_issues == 0 {
            println!("✅ 书签状态良好");
        } else {
            println!("\n💡 可使用 cleanup 命令清理");
        }
        
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }
}

/// 检测书签中的异常（仅检测，不删除）
pub fn detect_anomalies(bookmarks: &[Bookmark]) -> AnomalyReport {
    let mut report = AnomalyReport::default();
    let mut url_counts: HashMap<String, usize> = HashMap::new();
    
    collect_bookmark_stats(bookmarks, &mut url_counts, &mut report);
    
    // 检测重复URL
    for (_url, count) in &url_counts {
        if *count > 1 {
            report.duplicate_count += count - 1;
        }
    }
    
    // 检测空文件夹
    report.empty_folder_count = count_empty_folders(bookmarks);
    
    report
}

fn collect_bookmark_stats(
    bookmarks: &[Bookmark],
    url_counts: &mut HashMap<String, usize>,
    report: &mut AnomalyReport,
) {
    for bookmark in bookmarks {
        if bookmark.folder {
            collect_bookmark_stats(&bookmark.children, url_counts, report);
        } else if let Some(ref url) = bookmark.url {
            let normalized = normalize_url(url);
            *url_counts.entry(normalized).or_insert(0) += 1;
            
            // NSFW统计
            if is_nsfw_url(url, &bookmark.title) {
                report.nsfw_count += 1;
            }
        }
    }
}

fn count_empty_folders(bookmarks: &[Bookmark]) -> usize {
    let mut count = 0;
    for bookmark in bookmarks {
        if bookmark.folder {
            if bookmark.children.is_empty() {
                count += 1;
            } else {
                count += count_empty_folders(&bookmark.children);
            }
        }
    }
    count
}

/// 检测URL是否为NSFW内容（仅检测）
pub fn is_nsfw_url(url: &str, title: &str) -> bool {
    let url_lower = url.to_lowercase();
    let title_lower = title.to_lowercase();
    
    for pattern in NSFW_DOMAIN_PATTERNS {
        if url_lower.contains(pattern) { return true; }
    }
    
    for keyword in NSFW_TITLE_KEYWORDS {
        if title_lower.contains(keyword) { return true; }
    }
    
    false
}

fn normalize_url(url: &str) -> String {
    let mut normalized = url.trim().to_lowercase();
    if normalized.ends_with('/') { normalized.pop(); }
    if let Some(pos) = normalized.find('#') { normalized.truncate(pos); }
    normalized
}

// ============================================================
// 以下功能已被移除（误删风险太高）：
// - remove_bulk_imported_bookmarks
// - remove_history_pollution  
// - organize_nsfw_bookmarks
// - deep_clean_bookmarks
// ============================================================

/// 清理统计（保留结构用于兼容，自动清理功能已禁用）
#[derive(Debug, Default)]
#[allow(dead_code)]  // 保留用于API兼容性，自动清理功能已禁用
pub struct CleanupStats {
    pub bulk_removed: usize,
    pub history_removed: usize,
    pub nsfw_organized: usize,
    pub empty_removed: usize,
}

#[allow(dead_code)]  // 保留用于API兼容性
impl CleanupStats {
    pub fn total_removed(&self) -> usize {
        self.bulk_removed + self.history_removed + self.empty_removed
    }
    
    pub fn print_summary(&self) {
        println!("\n📊 清理统计");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("⚠️ 自动清理功能已禁用（误删风险）");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_is_nsfw_url() {
        assert!(is_nsfw_url("https://pornhub.com/video/123", "Video"));
        assert!(is_nsfw_url("https://exhentai.org/g/123/abc", "Gallery"));
        assert!(is_nsfw_url("https://example.com/page", "Hentai Collection"));
        assert!(!is_nsfw_url("https://github.com/user/repo", "Repository"));
    }
    
    #[test]
    fn test_normalize_url() {
        assert_eq!(normalize_url("https://example.com/"), "https://example.com");
        assert_eq!(normalize_url("HTTPS://EXAMPLE.COM"), "https://example.com");
    }
}
