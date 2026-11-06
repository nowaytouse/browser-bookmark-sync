use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, error, info};

/// 数据库安全操作包装器
///
/// 该模块提供了一种安全的方式来修改浏览器数据库，防止损坏。
/// 写入时持有 SQLite writer lock，先创建一致备份，再由 SQLite 事务负责原子提交或回滚。
/// 恢复时使用 SQLite online backup API，不在连接打开时手工搬动主库或 WAL 文件。
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
            // A write transaction is sufficient here; `PRAGMA locking_mode` returns rows and
            // must not be called through `execute`, which previously made every DB look locked.
            if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK;") {
                debug!(
                    "Failed to begin immediate transaction (db likely locked): {}",
                    e
                );
                return true;
            }
            false
        }
        Err(e) => {
            debug!("Failed to open database for locking check: {}", e);
            true
        }
    }
}

/// Restore a validated SQLite snapshot using SQLite's transactional backup API.
/// No database or WAL file is moved behind an open SQLite connection.
pub(crate) fn restore_database_snapshot(snapshot: &Path, db_path: &Path) -> Result<()> {
    check_compatibility(snapshot).context("Backup database validation failed")?;
    if is_database_locked(db_path) {
        return Err(anyhow!(
            "Database is locked by running browser. Please close the browser and try again."
        ));
    }

    let source = Connection::open_with_flags(snapshot, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut destination =
        Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    destination.busy_timeout(Duration::from_secs(5))?;
    let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
    let step = backup.step(-1)?;
    anyhow::ensure!(
        step == rusqlite::backup::StepResult::Done,
        "Database became busy while restoring ({step:?})"
    );
    drop(backup);

    let integrity: String =
        destination.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    anyhow::ensure!(
        integrity == "ok",
        "Restored database integrity check failed: {integrity}"
    );
    fs::File::open(db_path)?.sync_all()?;
    Ok(())
}

/// Safely execute one SQLite transaction with a mandatory consistent backup.
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

    // Hold SQLite's writer lock from backup through commit. This blocks a browser from
    // committing between those steps without externally moving its WAL files.
    let source = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .context("Failed to open database for safe write")?;
    source
        .busy_timeout(Duration::from_secs(5))
        .context("Failed to configure database lock timeout")?;
    source
        .execute_batch("BEGIN IMMEDIATE;")
        .context("Failed to acquire SQLite writer lock")?;

    let parent = db_path
        .parent()
        .ok_or_else(|| anyhow!("Invalid database path"))?;
    let db_name = db_path
        .file_name()
        .ok_or_else(|| anyhow!("Invalid database path"))?
        .to_string_lossy();
    let backup_path = parent.join(format!("{db_name}.backup-{}", uuid::Uuid::new_v4()));

    let result = (|| -> Result<()> {
        debug!("Creating consistent SQLite backup at {:?}", backup_path);
        let snapshot_source =
            Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        snapshot_source
            .execute("VACUUM main INTO ?1", [backup_path.to_string_lossy()])
            .context("Failed to create consistent database backup")?;
        fs::File::open(&backup_path)?.sync_all()?;

        operation(&source)?;
        let integrity: String = source
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .context("Failed to verify modified database")?;
        anyhow::ensure!(
            integrity == "ok",
            "Database integrity check failed after modification: {integrity}"
        );
        source.execute_batch("COMMIT;")?;
        fs::File::open(db_path)?.sync_all()?;
        info!(
            "✅ Database transaction committed; backup: {:?}",
            backup_path
        );
        Ok(())
    })();

    if let Err(ref error) = result {
        let _ = source.execute_batch("ROLLBACK;");
        error!("❌ Operation failed; original database remains usable: {error}");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_write_is_atomic_and_keeps_a_unique_backup() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("places.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE bookmarks (url TEXT NOT NULL);
             INSERT INTO bookmarks VALUES ('https://before.example');",
        )
        .unwrap();
        drop(conn);

        safe_write_transaction(&db_path, |copy| {
            copy.execute("INSERT INTO bookmarks VALUES ('https://after.example')", [])?;
            Ok(())
        })
        .unwrap();

        let updated = Connection::open(&db_path).unwrap();
        let count: i64 = updated
            .query_row("SELECT COUNT(*) FROM bookmarks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        let backups: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("places.sqlite.backup-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        let backup = Connection::open(backups[0].path()).unwrap();
        let backup_count: i64 = backup
            .query_row("SELECT COUNT(*) FROM bookmarks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(backup_count, 1);
    }

    #[test]
    fn concurrent_writer_cannot_commit_during_safe_transaction() {
        use std::sync::mpsc::sync_channel;
        use std::thread;

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("places.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE bookmarks (url TEXT NOT NULL);
             INSERT INTO bookmarks VALUES ('https://before.example');",
        )
        .unwrap();
        drop(conn);

        let (start_writer, wait_for_start) = sync_channel(0);
        let (writer_done, wait_for_writer) = sync_channel(0);
        let writer_path = db_path.clone();
        let writer = thread::spawn(move || {
            wait_for_start.recv().unwrap();
            let writer_conn = Connection::open(writer_path).unwrap();
            writer_conn.busy_timeout(Duration::ZERO).unwrap();
            let outcome = match writer_conn.execute(
                "INSERT INTO bookmarks VALUES ('https://concurrent.example')",
                [],
            ) {
                Ok(_) => Err("concurrent writer unexpectedly committed".to_string()),
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if matches!(
                        error.code,
                        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                    ) =>
                {
                    Ok(())
                }
                Err(error) => Err(format!("concurrent writer failed unexpectedly: {error}")),
            };
            writer_done.send(outcome).unwrap();
        });

        safe_write_transaction(&db_path, |copy| {
            start_writer.send(()).unwrap();
            wait_for_writer
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| anyhow!("concurrent writer did not finish: {error}"))?
                .map_err(anyhow::Error::msg)?;
            copy.execute("INSERT INTO bookmarks VALUES ('https://after.example')", [])?;
            Ok(())
        })
        .unwrap();
        writer.join().unwrap();

        let updated = Connection::open(&db_path).unwrap();
        let mut statement = updated
            .prepare("SELECT url FROM bookmarks ORDER BY url")
            .unwrap();
        let urls: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            urls,
            vec![
                "https://after.example".to_string(),
                "https://before.example".to_string(),
            ]
        );
    }

    #[test]
    fn failed_operation_rolls_back_all_changes() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("places.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE bookmarks (url TEXT NOT NULL)", [])
            .unwrap();
        drop(conn);

        let error = safe_write_transaction(&db_path, |connection| {
            connection.execute(
                "INSERT INTO bookmarks VALUES ('https://must-rollback.example')",
                [],
            )?;
            anyhow::bail!("test failure")
        })
        .unwrap_err();
        assert!(error.to_string().contains("test failure"));

        let connection = Connection::open(db_path).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM bookmarks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
