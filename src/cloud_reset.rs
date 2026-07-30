//! Cloud reset module - reserved for future cloud sync feature

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::{info, warn};

/// 清空Firefox/Waterfox的本地书签（保留根文件夹）
pub fn clear_local_bookmarks(db_path: &Path) -> Result<()> {
    info!("🗑️  Clearing local bookmarks...");

    // 使用WAL模式和超时
    let conn = Connection::open(db_path).context("Failed to open places.sqlite")?;

    conn.busy_timeout(std::time::Duration::from_secs(30))?;

    // 先检查当前书签数量
    let before_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 1",
            [],
            |row| row.get(0),
        )
        .context("查询清空前的书签数量失败")?;

    info!("   Current bookmarks: {}", before_count);

    // 删除所有非根书签（type=1是书签，type=2是文件夹）
    // 保留根文件夹：1=root, 2=menu, 3=toolbar, 4=tags, 5=unfiled, 6=mobile
    let deleted = conn
        .execute("DELETE FROM moz_bookmarks WHERE id > 6", [])
        .context("Failed to delete bookmarks")?;

    info!("   Deleted {} bookmark entries", deleted);

    // 清理moz_places中的孤立记录
    let orphans = conn.execute(
        "DELETE FROM moz_places WHERE id NOT IN (SELECT DISTINCT fk FROM moz_bookmarks WHERE fk IS NOT NULL) AND id NOT IN (SELECT DISTINCT place_id FROM moz_historyvisits)",
        [],
    ).context("清理 moz_places 孤立记录失败")?;

    info!("   Cleaned {} orphan places", orphans);

    // 验证清空
    let after_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 1",
            [],
            |row| row.get(0),
        )
        .context("查询清空后的书签数量失败")?;

    info!("   After cleanup: {} bookmarks", after_count);

    if after_count < 10 {
        info!("✅ Local bookmarks cleared successfully");
    } else {
        warn!("⚠️  Some bookmarks may remain: {}", after_count);
    }

    Ok(())
}

/// Wait for user to confirm cloud sync is complete
pub fn wait_for_cloud_sync() -> Result<()> {
    info!("");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📤 Please follow these steps:");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("");
    info!("   1. Launch Waterfox");
    info!("   2. Wait for sync icon to spin and stop (~30 seconds)");
    info!("   3. Confirm bookmark bar is empty");
    info!("   4. Close Waterfox");
    info!("");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("");

    print!("Press Enter when done...");
    use std::io::{self, Write};
    if let Err(e) = io::stdout().flush() {
        tracing::debug!("标准输出缓冲区刷新失败: {}", e);
    }

    let mut input = String::new();
    if let Err(e) = io::stdin().read_line(&mut input) {
        tracing::warn!("读取用户终端输入失败: {}", e);
    }

    info!("✅ Continuing...");

    Ok(())
}

/// 验证本地书签已清空
pub fn verify_cleared(db_path: &Path) -> Result<bool> {
    let conn = Connection::open(db_path).context("Failed to open places.sqlite")?;

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 1",
            [],
            |row| row.get(0),
        )
        .context("查询验证本地书签数量失败")?;

    info!("📊 Current bookmark count: {}", count);

    Ok(count < 10) // 允许少量系统书签
}
