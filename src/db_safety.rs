use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use tracing::{debug, error, info, warn};

/// 数据库安全操作包装器
///
/// 该模块提供了一种安全的方式来修改浏览器数据库，防止损坏。
/// 它实现了“复制-验证-替换”模式：
/// 1. 将目标数据库复制到临时位置
/// 2. 在副本上执行写入操作
/// 3. 验证副本的完整性
/// 4. 仅当验证通过时，才用副本替换原始数据库
/// 5. 如果出现任何错误，原始数据库保持不变
///
/// 检查数据库兼容性，验证数据库是否可以被当前的 SQLite 版本安全打开
pub fn check_compatibility(db_path: &Path) -> Result<()> {
    if !db_path.exists() {
        return Err(anyhow!("Database file does not exist: {db_path:?}"));
    }

    // 尝试以只读模式打开
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .context("Failed to open database for compatibility check")?;

    // 检查 SQLite 版本
    let version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .context("Failed to query SQLite version")?;

    debug!("Database SQLite version: {}", version);

    // 检查架构版本 (user_version)
    let schema_version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("Failed to query schema version")?;

    debug!("Database schema version: {}", schema_version);

    // 运行快速完整性检查
    let integrity: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .context("Failed to run integrity check")?;

    if integrity != "ok" {
        return Err(anyhow!("Database integrity check failed: {integrity}"));
    }

    Ok(())
}

/// 检查数据库是否被锁定（浏览器是否正在运行）
/// 通过检查 .lock 文件或尝试获取排他锁
pub fn is_database_locked(db_path: &Path) -> bool {
    // 1. 检查是否存在同名的 .lock 文件 (常见于 Firefox)
    // Firefox 使用 places.sqlite-wal 和 places.sqlite-shm，但也可能锁定主文件
    let file_name = db_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !file_name.is_empty() {
        let parent = db_path.parent().unwrap_or_else(|| Path::new("."));

        // 检查常见的锁定文件
        let lock_files = vec![
            format!("{}.lock", file_name),
            "lock".to_string(),
            ".parentlock".to_string(),
        ];

        for lock_file in lock_files {
            let lock_path = parent.join(lock_file);
            if lock_path.exists() {
                debug!("Found lock file: {:?}", lock_path);
                return true;
            }
        }
    }

    // 2. 尝试以读写模式打开并获取排他锁
    // 注意：这可能会失败，如果数据库被其他进程使用
    match Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE) {
        Ok(conn) => {
            // 尝试设置锁定模式为 EXCLUSIVE
            if let Err(e) = conn.execute("PRAGMA locking_mode = EXCLUSIVE", []) {
                debug!("Failed to set locking mode: {}", e);
                return true;
            }
            // 尝试开始一个立即事务
            if let Err(e) = conn.execute("BEGIN IMMEDIATE", []) {
                debug!(
                    "Failed to begin immediate transaction (db likely locked): {}",
                    e
                );
                return true;
            }
            // 如果成功，回滚并关闭
            if let Err(e) = conn.execute("ROLLBACK", []) {
                tracing::warn!("尝试对锁检测数据库连接执行 ROLLBACK 失败: {}", e);
            }
            false
        }
        Err(e) => {
            debug!("Failed to open database for locking check: {}", e);
            true
        }
    }
}

/// 安全地执行写入事务
/// 使用“复制-验证-替换”模式防止数据损坏
pub fn safe_write_transaction<F>(db_path: &Path, operation: F) -> Result<()>
where
    F: FnOnce(&Connection) -> Result<()>,
{
    info!("🛡️  Starting safe database transaction for {:?}", db_path);

    // 1. 预检查
    if is_database_locked(db_path) {
        return Err(anyhow!(
            "Database is locked by running browser. Please close the browser and try again."
        ));
    }

    check_compatibility(db_path).context("Database compatibility check failed")?;

    // 2. 创建临时副本
    let temp_dir = std::env::temp_dir().join("browser-sync-safe-write");
    fs::create_dir_all(&temp_dir)?;

    let db_name = db_path
        .file_name()
        .ok_or_else(|| anyhow!("Invalid database path"))?
        .to_string_lossy();

    let temp_db_path = temp_dir.join(format!("{}_{}.tmp", db_name, uuid::Uuid::new_v4()));

    debug!("Creating temporary copy at {:?}", temp_db_path);
    fs::copy(db_path, &temp_db_path).context("Failed to create temporary database copy")?;

    // 还要复制 WAL 和 SHM 文件（如果存在）
    let wal_path = db_path.with_extension("sqlite-wal");
    let shm_path = db_path.with_extension("sqlite-shm");

    if wal_path.exists() {
        if let Err(e) = fs::copy(&wal_path, temp_db_path.with_extension("sqlite-wal")) {
            tracing::warn!(
                "复制 SQLite WAL 日志文件 '{}' 失败: {}，可能导致事务状态快照不完整",
                wal_path.display(),
                e
            );
        }
    }
    if shm_path.exists() {
        if let Err(e) = fs::copy(&shm_path, temp_db_path.with_extension("sqlite-shm")) {
            tracing::warn!(
                "复制 SQLite SHM 内存映射文件 '{}' 失败: {}，可能影响临时数据库快照连接",
                shm_path.display(),
                e
            );
        }
    }

    // 3. 在副本上执行操作
    let result = (|| -> Result<()> {
        let conn = Connection::open(&temp_db_path).context("Failed to open temporary database")?;

        // 执行用户提供的操作
        operation(&conn)?;

        // 4. 验证完整性
        debug!("Verifying integrity of modified database...");
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .context("Failed to verify integrity")?;

        if integrity != "ok" {
            return Err(anyhow!(
                "Database integrity check failed after modification: {integrity}"
            ));
        }

        // 显式关闭连接以确保所有写入都已刷新
        drop(conn);
        Ok(())
    })();

    // 5. 根据结果处理
    match result {
        Ok(()) => {
            info!("✅ Operation successful and verified. Replacing original database.");

            // 备份原始数据库（作为额外的安全措施）
            let backup_path = db_path.with_extension("sqlite.bak");
            if let Err(e) = fs::copy(db_path, &backup_path) {
                warn!("Failed to create backup of original database: {}", e);
                // 继续，因为我们已经验证了新数据库
            } else {
                debug!("Created backup at {:?}", backup_path);
            }

            // 替换原始数据库
            // 使用 rename 原子替换（如果可能）
            if let Err(e) = fs::rename(&temp_db_path, db_path) {
                // 如果跨文件系统重命名失败，尝试复制并删除
                debug!("Rename failed ({}), trying copy-and-delete...", e);
                fs::copy(&temp_db_path, db_path).context("Failed to replace original database")?;
                fs::remove_file(&temp_db_path)?;
            }

            // 清理临时 WAL/SHM 文件
            let temp_wal = temp_db_path.with_extension("sqlite-wal");
            let temp_shm = temp_db_path.with_extension("sqlite-shm");
            if temp_wal.exists() {
                if let Err(e) = fs::remove_file(&temp_wal) {
                    tracing::debug!("清理临时 WAL 日志文件 '{}' 失败: {}", temp_wal.display(), e);
                }
            }
            if temp_shm.exists() {
                if let Err(e) = fs::remove_file(&temp_shm) {
                    tracing::debug!("清理临时 SHM 文件 '{}' 失败: {}", temp_shm.display(), e);
                }
            }

            Ok(())
        }
        Err(e) => {
            error!("❌ Operation failed or validation failed. Original database unchanged.");
            error!("Error: {}", e);

            // 清理临时文件
            if let Err(e) = fs::remove_file(&temp_db_path) {
                tracing::debug!(
                    "发生错误后清理临时主数据库文件 '{}' 失败: {}",
                    temp_db_path.display(),
                    e
                );
            }
            let temp_wal = temp_db_path.with_extension("sqlite-wal");
            let temp_shm = temp_db_path.with_extension("sqlite-shm");
            if temp_wal.exists() {
                if let Err(e) = fs::remove_file(&temp_wal) {
                    tracing::debug!(
                        "发生错误后清理临时 WAL 文件 '{}' 失败: {}",
                        temp_wal.display(),
                        e
                    );
                }
            }
            if temp_shm.exists() {
                if let Err(e) = fs::remove_file(&temp_shm) {
                    tracing::debug!(
                        "发生错误后清理临时 SHM 文件 '{}' 失败: {}",
                        temp_shm.display(),
                        e
                    );
                }
            }

            Err(e)
        }
    }
}
