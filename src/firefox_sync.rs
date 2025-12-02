//! Firefox Sync integration module - reserved for future cloud sync feature
#![allow(dead_code)]
#![allow(clippy::single_component_path_imports)]

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Firefox Sync配置
pub struct FirefoxSyncConfig {
    pub profile_path: PathBuf,
    pub sync_enabled: bool,
    pub sync_username: Option<String>,
}

impl FirefoxSyncConfig {
    /// 检测Firefox Sync配置
    pub fn detect(profile_path: &Path) -> Result<Self> {
        let prefs_path = profile_path.join("prefs.js");

        if !prefs_path.exists() {
            return Ok(Self {
                profile_path: profile_path.to_path_buf(),
                sync_enabled: false,
                sync_username: None,
            });
        }

        let content = fs::read_to_string(&prefs_path).context("Failed to read prefs.js")?;

        // 检查是否启用了Sync
        let sync_enabled = content.contains("services.sync.username");

        // 提取用户名
        let sync_username = if sync_enabled {
            content
                .lines()
                .find(|line| line.contains("services.sync.username"))
                .and_then(|line| {
                    // 提取 "username" 部分
                    line.split('"').nth(3).map(|s| s.to_string())
                })
        } else {
            None
        };

        Ok(Self {
            profile_path: profile_path.to_path_buf(),
            sync_enabled,
            sync_username,
        })
    }

    /// 触发立即同步
    ///
    /// 方法：修改prefs.js，设置nextSync=0，强制浏览器启动时立即同步
    pub fn trigger_immediate_sync(&self) -> Result<()> {
        if !self.sync_enabled {
            debug!("Firefox Sync not enabled, skipping");
            return Ok(());
        }

        info!("🔄 Triggering Firefox Sync...");

        let prefs_path = self.profile_path.join("prefs.js");
        let content = fs::read_to_string(&prefs_path).context("Failed to read prefs.js")?;

        // 修改nextSync为0（立即同步）
        let new_content = if content.contains("services.sync.nextSync") {
            // 替换现有值
            content
                .lines()
                .map(|line| {
                    if line.contains("services.sync.nextSync") {
                        r#"user_pref("services.sync.nextSync", 0);"#.to_string()
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            // 添加新配置
            format!("{}\nuser_pref(\"services.sync.nextSync\", 0);\n", content)
        };

        // 写回文件
        fs::write(&prefs_path, new_content).context("Failed to write prefs.js")?;

        info!("   ✅ Firefox Sync will trigger on next browser start");

        Ok(())
    }

    /// 等待同步完成
    ///
    /// 方法：监控places.sqlite的修改时间，如果在一定时间内没有变化，认为同步完成
    pub fn wait_for_sync_complete(&self, timeout_secs: u64) -> Result<bool> {
        use std::thread;
        use std::time::{Duration, SystemTime};

        if !self.sync_enabled {
            return Ok(true);
        }

        info!(
            "⏳ Waiting for Firefox Sync to complete (timeout: {}s)...",
            timeout_secs
        );

        let db_path = self.profile_path.join("places.sqlite");
        let start_time = SystemTime::now();
        let timeout = Duration::from_secs(timeout_secs);

        let mut last_modified = fs::metadata(&db_path)?.modified()?;
        let mut stable_count = 0;

        loop {
            thread::sleep(Duration::from_secs(2));

            let current_modified = fs::metadata(&db_path)?.modified()?;

            if current_modified == last_modified {
                stable_count += 1;
                if stable_count >= 3 {
                    // 连续3次检查（6秒）没有变化，认为同步完成
                    info!("   ✅ Sync appears to be complete");
                    return Ok(true);
                }
            } else {
                // 文件还在变化，重置计数
                stable_count = 0;
                last_modified = current_modified;
                debug!("   Database still changing...");
            }

            if start_time.elapsed()? > timeout {
                warn!("   ⚠️  Timeout waiting for sync");
                return Ok(false);
            }
        }
    }

    /// 显示警告信息
    pub fn show_warning(&self) {
        if !self.sync_enabled {
            return;
        }

        warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        warn!("⚠️  Firefox Sync Detected");
        warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        warn!("");
        warn!("   Firefox Sync is enabled for this profile");
        if let Some(username) = &self.sync_username {
            warn!("   Account: {}", username);
        }
        warn!("");
        warn!("   ⚠️  Important:");
        warn!("   - Local changes will be synced to cloud");
        warn!("   - Cloud data may overwrite local changes");
        warn!("   - Sync will be triggered after modifications");
        warn!("");
        warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        warn!("");
    }
}

/// 同步策略
pub enum SyncStrategy {
    /// 禁用Sync检测（默认行为）
    Ignore,

    /// 警告用户但继续
    WarnAndContinue,

    /// 触发立即同步
    TriggerSync,

    /// 触发同步并等待完成
    TriggerAndWait { timeout_secs: u64 },

    /// 使用Firefox Sync API直接上传到云端（推荐）
    UseAPI,
}

/// Firefox Sync处理器
pub struct FirefoxSyncHandler {
    config: FirefoxSyncConfig,
    strategy: SyncStrategy,
}

impl FirefoxSyncHandler {
    /// 创建处理器
    pub fn new(profile_path: &Path, strategy: SyncStrategy) -> Result<Self> {
        let config = FirefoxSyncConfig::detect(profile_path)?;

        Ok(Self { config, strategy })
    }

    /// 在写入前执行
    pub fn before_write(&self) -> Result<()> {
        match self.strategy {
            SyncStrategy::Ignore => {
                // 不做任何处理
                Ok(())
            }
            SyncStrategy::WarnAndContinue => {
                self.config.show_warning();
                Ok(())
            }
            SyncStrategy::TriggerSync
            | SyncStrategy::TriggerAndWait { .. }
            | SyncStrategy::UseAPI => {
                self.config.show_warning();
                Ok(())
            }
        }
    }

    /// 在写入后执行
    pub fn after_write(&self) -> Result<()> {
        match self.strategy {
            SyncStrategy::Ignore | SyncStrategy::WarnAndContinue => Ok(()),
            SyncStrategy::TriggerSync => {
                self.config.trigger_immediate_sync()?;
                info!("");
                info!("📝 Next steps:");
                info!("   1. Start Waterfox");
                info!("   2. Firefox Sync will automatically upload changes to cloud");
                info!("   3. Wait for sync to complete (check sync icon)");
                info!("");
                Ok(())
            }
            SyncStrategy::TriggerAndWait { timeout_secs } => {
                self.config.trigger_immediate_sync()?;

                info!("");
                info!("📝 Please start Waterfox now to trigger sync...");
                info!("   (Press Enter when browser is started)");
                info!("");

                // 等待用户启动浏览器
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();

                // 等待同步完成
                let completed = self.config.wait_for_sync_complete(timeout_secs)?;

                if completed {
                    info!("✅ Firefox Sync completed successfully");
                } else {
                    warn!("⚠️  Sync may not be complete, please verify manually");
                }

                Ok(())
            }
            SyncStrategy::UseAPI => {
                // 使用API策略在sync.rs中处理
                Ok(())
            }
        }
    }

    /// 检查是否启用了Sync
    #[allow(dead_code)] // 公开API，预留给未来使用
    pub fn is_sync_enabled(&self) -> bool {
        self.config.sync_enabled
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_sync_detection() {
        // 测试需要真实的profile路径
        // 这里只是示例
    }
}
