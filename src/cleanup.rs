//! 书签清理模块 - 检测异常书签数据（仅检测，不自动删除）
//! 
//! 功能：
//! 1. 检测批量导入的异常书签（同一时间戳大量添加）
//! 2. 检测历史记录污染（URL模式匹配）
//! 3. 检测重复书签
//! 4. 检测NSFW内容
//! 5. 检测空文件夹
//!
//! ⚠️ 注意：本模块仅提供检测功能，不自动删除任何书签
//! 自动删除功能已被移除，因为误删风险太高

use std::collections::HashMap;

use crate::browsers::Bookmark;

/// 异常检测结果
#[derive(Debug, Default)]
pub struct AnomalyReport {
    /// 批量导入的书签数量（同一秒内添加超过100个）
    pub bulk_import_count: usize,
    /// 批量导入的时间戳
    pub bulk_import_timestamps: Vec<(i64, usize)>,
    /// 重复URL数量
    pub duplicate_count: usize,
    /// 疑似历史记录的书签数量
    pub history_pollution_count: usize,
    /// NSFW内容数量
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
        println!("\n📊 异常检测报告");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        if self.bulk_import_count > 0 {
            println!("⚠️  批量导入异常: {} 个书签", self.bulk_import_count);
            for (ts, count) in &self.bulk_import_timestamps {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| format!("timestamp: {}", ts));
                println!("   - {} : {} 个书签", datetime, count);
            }
        }
        
        if self.duplicate_count > 0 {
            println!("🔄 重复URL: {} 个", self.duplicate_count);
        }
        
        if self.history_pollution_count > 0 {
            println!("📜 疑似历史记录污染: {} 个", self.history_pollution_count);
        }
        
        if self.empty_folder_count > 0 {
            println!("📁 空文件夹: {} 个", self.empty_folder_count);
        }
        
        // NSFW不是问题，只是统计信息
        if self.nsfw_count > 0 {
            println!("ℹ️  NSFW内容: {} 个（仅统计）", self.nsfw_count);
        }
        
        // 只有这些才算问题
        let total_issues = self.bulk_import_count + self.duplicate_count 
            + self.history_pollution_count + self.empty_folder_count;
        
        if total_issues == 0 {
            println!("✅ 书签状态良好");
        } else {
            println!("\n发现 {} 个可能需要关注的项目", total_issues);
        }
        
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }
}

/// 检测书签中的异常（仅检测，不删除）
pub fn detect_anomalies(bookmarks: &[Bookmark]) -> AnomalyReport {
    let mut report = AnomalyReport::default();
    let mut timestamp_counts: HashMap<i64, usize> = HashMap::new();
    let mut url_counts: HashMap<String, usize> = HashMap::new();
    
    collect_bookmark_stats(bookmarks, &mut timestamp_counts, &mut url_counts, &mut report);
    
    // 检测批量导入（同一秒内超过100个书签）
    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    
    for (ts, count) in &timestamp_counts {
        if *ts > one_hour_ago { continue; }
        if *count > 100 {
            report.bulk_import_count += count;
            report.bulk_import_timestamps.push((*ts, *count));
        }
    }
    
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
    timestamp_counts: &mut HashMap<i64, usize>,
    url_counts: &mut HashMap<String, usize>,
    report: &mut AnomalyReport,
) {
    for bookmark in bookmarks {
        if bookmark.folder {
            collect_bookmark_stats(&bookmark.children, timestamp_counts, url_counts, report);
        } else {
            if let Some(ts) = bookmark.date_added {
                let ts_second = if ts > 1_000_000_000_000_000 {
                    ts / 1_000_000
                } else if ts > 1_000_000_000_000 {
                    ts / 1_000
                } else {
                    ts
                };
                *timestamp_counts.entry(ts_second).or_insert(0) += 1;
            }
            
            if let Some(ref url) = bookmark.url {
                let normalized = normalize_url(url);
                *url_counts.entry(normalized).or_insert(0) += 1;
                
                // 仅统计，不删除
                if is_nsfw_url(url, &bookmark.title) {
                    report.nsfw_count += 1;
                }
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

/// 清理统计（保留结构用于兼容）
#[derive(Debug, Default)]
pub struct CleanupStats {
    pub bulk_removed: usize,
    pub history_removed: usize,
    pub nsfw_organized: usize,
    pub empty_removed: usize,
}

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
