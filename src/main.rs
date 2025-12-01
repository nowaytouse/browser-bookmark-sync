use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{info, warn};

mod browsers;
mod sync;
mod scheduler;
mod validator;
mod firefox_sync;
mod firefox_sync_api;
mod cloud_reset;
mod cleanup;
mod browser_utils;

use sync::SyncEngine;

#[derive(Parser)]
#[command(name = "browser-bookmark-sync")]
#[command(about = "🔖 跨浏览器书签管理工具 - 合并、去重、导出", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 📋 列出所有检测到的浏览器及其书签位置
    #[command(alias = "l", alias = "ls")]
    List,
    
    /// 📤 导出书签到HTML文件 (推荐方式 - 避免同步覆盖)
    #[command(alias = "export", alias = "e")]
    ExportHtml {
        /// 输出HTML文件路径
        #[arg(short = 'o', long, default_value = "~/Desktop/bookmarks_export.html")]
        output: String,
        
        /// 来源浏览器 (逗号分隔, 默认: all)
        #[arg(short = 'b', long, default_value = "all")]
        browsers: String,
        
        /// 合并所有书签到扁平结构 (不按浏览器分文件夹)
        #[arg(long)]
        merge: bool,
        
        /// 去除重复书签
        #[arg(long, short = 'd')]
        deduplicate: bool,
        
        /// 同时导入已有HTML备份文件
        #[arg(long)]
        include_html: Option<String>,
        
        /// 详细输出
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// 🔍 验证书签完整性
    #[command(alias = "v", alias = "check")]
    Validate {
        /// 显示详细验证报告
        #[arg(short, long)]
        detailed: bool,
    },
    
    /// 🧹 清理书签 (去重复/删除空文件夹)
    #[command(alias = "c", alias = "clean")]
    Cleanup {
        /// 目标浏览器 (逗号分隔, 默认: all)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// 删除重复书签
        #[arg(long)]
        remove_duplicates: bool,
        
        /// 删除空文件夹
        #[arg(long)]
        remove_empty_folders: bool,
        
        /// 预览模式 - 不实际修改
        #[arg(short, long)]
        dry_run: bool,
        
        /// 详细输出
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// 🧠 智能分类书签 (按URL模式自动归类)
    #[command(alias = "so", alias = "smart")]
    SmartOrganize {
        /// 目标浏览器 (逗号分隔, 默认: all)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
        
        /// 自定义规则文件 (JSON格式)
        #[arg(short = 'r', long)]
        rules_file: Option<String>,
        
        /// 只处理未分类的书签
        #[arg(long)]
        uncategorized_only: bool,
        
        /// 显示规则匹配统计
        #[arg(long)]
        show_stats: bool,
        
        /// 预览模式 - 不实际修改
        #[arg(short, long)]
        dry_run: bool,
        
        /// 详细输出
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// 📜 列出可用的分类规则
    ListRules,
    
    /// 🔄 同步浏览器历史记录 (双向增量同步)
    #[command(alias = "sh", alias = "history")]
    SyncHistory {
        /// Hub浏览器 (逗号分隔, 默认: waterfox,brave-nightly)
        #[arg(short = 'b', long, default_value = "waterfox,brave-nightly")]
        browsers: String,
        
        /// 同步天数 (默认: 30天)
        #[arg(short = 'd', long, default_value = "30")]
        days: i32,
        
        /// 预览模式 - 不实际修改
        #[arg(long)]
        dry_run: bool,
        
        /// 详细输出
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// 🔍 分析书签 (NSFW检测)
    #[command(alias = "a")]
    Analyze {
        /// 目标浏览器 (逗号分隔, 默认: all)
        #[arg(short = 'b', long)]
        browsers: Option<String>,
    },
    
    /// 💾 创建主备份 (合并所有浏览器数据)
    MasterBackup {
        /// 输出目录
        #[arg(short = 'o', long, default_value = "~/Desktop/BookmarkBackup")]
        output: String,
        
        /// 包含完整数据 (不只是唯一URL)
        #[arg(long)]
        include_full: bool,
    },
    
    /// 🔄 恢复书签备份
    RestoreBackup {
        /// 要恢复的浏览器 (如: waterfox)
        #[arg(short = 'b', long)]
        browser: String,
        
        /// 备份文件路径 (可选, 默认使用最新备份)
        #[arg(short = 'f', long)]
        file: Option<String>,
    },
    
    /// 🗑️ 清空浏览器书签 (调试用 - 谨慎使用!)
    #[command(alias = "clear")]
    ClearBookmarks {
        /// 目标浏览器 (逗号分隔)
        #[arg(short = 'b', long)]
        browsers: String,
        
        /// 跳过确认
        #[arg(short = 'y', long)]
        yes: bool,
        
        /// 预览模式
        #[arg(short, long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            info!("📋 检测浏览器...");
            let engine = SyncEngine::new()?;
            engine.list_browsers()?;
        }
        
        Commands::ExportHtml { output, browsers, merge, deduplicate, include_html, verbose } => {
            info!("📤 导出书签到HTML文件");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("📄 输出: {}", output);
            info!("🌐 来源: {}", browsers);
            if merge { info!("🔀 合并模式"); }
            if deduplicate { info!("🧹 去重复"); }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            
            let engine = SyncEngine::new()?;
            
            let mut extra_bookmarks: Vec<crate::browsers::Bookmark> = Vec::new();
            if let Some(html_path) = &include_html {
                let expanded = if html_path.starts_with("~/") {
                    html_path.replacen("~", &std::env::var("HOME").unwrap_or_default(), 1)
                } else { html_path.clone() };
                
                info!("📥 导入HTML: {}", expanded);
                match sync::import_bookmarks_from_html(&expanded) {
                    Ok(bookmarks) => {
                        let count = bookmarks.iter().map(|b| count_bookmark_tree(b)).sum::<usize>();
                        info!("  ✅ {} 书签", count);
                        extra_bookmarks = bookmarks;
                    }
                    Err(e) => warn!("  ⚠️ 导入失败: {}", e),
                }
            }
            
            let count = engine.export_to_html_with_extra(
                Some(&browsers), &output, merge, deduplicate, verbose, extra_bookmarks
            ).await?;
            
            info!("\n🎉 导出完成! {} 书签", count);
            info!("💡 请手动导入到目标浏览器，避免被同步覆盖");
        }
        
        Commands::Validate { detailed } => {
            info!("🔍 验证书签...");
            let engine = SyncEngine::new()?;
            let report = engine.validate(detailed)?;
            println!("{}", report);
        }
        
        Commands::Cleanup { browsers, remove_duplicates, remove_empty_folders, dry_run, verbose } => {
            if !remove_duplicates && !remove_empty_folders {
                eprintln!("⚠️ 请指定清理选项: --remove-duplicates 或 --remove-empty-folders");
                std::process::exit(1);
            }
            
            info!("🧹 清理书签");
            let mut engine = SyncEngine::new()?;
            engine.cleanup_bookmarks(
                browsers.as_deref(), remove_duplicates, remove_empty_folders, dry_run, verbose
            ).await?;
            info!("✅ 清理完成!");
        }
        
        Commands::SmartOrganize { browsers, rules_file, uncategorized_only, show_stats, dry_run, verbose } => {
            info!("🧠 智能分类书签");
            let mut engine = SyncEngine::new()?;
            engine.smart_organize(
                browsers.as_deref(), rules_file.as_deref(), uncategorized_only, show_stats, dry_run, verbose
            ).await?;
            info!("✅ 分类完成!");
        }
        
        Commands::ListRules => {
            SyncEngine::print_builtin_rules();
        }
        
        Commands::SyncHistory { browsers, days, dry_run, verbose } => {
            info!("🔄 同步浏览器历史记录");
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("🌐 Hub浏览器: {}", browsers);
            info!("📅 同步范围: 最近{}天", days);
            if dry_run { info!("🏃 预览模式"); }
            info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            
            let mut engine = SyncEngine::new()?;
            engine.sync_history(Some(days), dry_run, verbose).await?;
            info!("✅ 历史记录同步完成!");
        }
        
        Commands::Analyze { browsers } => {
            info!("🔍 分析书签...");
            let engine = SyncEngine::new()?;
            engine.analyze_bookmarks(browsers.as_deref()).await?;
        }
        
        Commands::MasterBackup { output, include_full } => {
            info!("💾 创建主备份...");
            sync::create_master_backup(&output, include_full).await?;
            info!("✅ 备份完成!");
        }
        
        Commands::RestoreBackup { browser, file } => {
            info!("🔄 恢复备份...");
            let mut engine = SyncEngine::new()?;
            engine.restore_backup(&browser, file.as_deref()).await?;
            info!("✅ 恢复完成!");
        }
        
        Commands::ClearBookmarks { browsers, yes, dry_run } => {
            info!("🗑️ 清空浏览器书签");
            info!("⚠️ 警告: 此操作将清空所有书签!");
            info!("🎯 目标: {}", browsers);
            
            if !yes && !dry_run {
                print!("确认? (y/N): ");
                use std::io::{self, Write};
                io::stdout().flush().ok();
                let mut input = String::new();
                io::stdin().read_line(&mut input).ok();
                if !input.trim().eq_ignore_ascii_case("y") {
                    info!("❌ 已取消");
                    return Ok(());
                }
            }
            
            let mut engine = SyncEngine::new()?;
            engine.clear_bookmarks(&browsers, dry_run).await?;
            info!("✅ 完成!");
        }
    }

    Ok(())
}

fn count_bookmark_tree(bookmark: &crate::browsers::Bookmark) -> usize {
    let mut count = if bookmark.url.is_some() { 1 } else { 0 };
    for child in &bookmark.children {
        count += count_bookmark_tree(child);
    }
    count
}
