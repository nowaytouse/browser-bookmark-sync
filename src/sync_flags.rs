use anyhow::{anyhow, Result};
use tracing::warn;

/// 同步标志配置
/// 控制哪些数据类型被同步或导出
#[derive(Debug, Clone)]
pub struct SyncFlags {
    /// 同步书签
    pub bookmarks: bool,

    /// 同步浏览历史
    pub history: bool,

    /// 同步阅读列表 (Safari, Firefox)
    pub reading_list: bool,

    /// 同步 Cookie (⚠️ 影响会话)
    pub cookies: bool,

    /// 同步密码 (⚠️ 安全风险，始终禁用)
    pub passwords: bool,

    /// 同步扩展程序 (⚠️ 不可行，始终禁用)
    pub extensions: bool,

    /// 历史记录天数限制 (None = 全部)
    pub history_days: Option<i32>,

    /// 是否去重
    pub deduplicate: bool,

    /// 是否合并文件夹结构
    pub merge: bool,

    /// 详细输出
    pub verbose: bool,
}

impl Default for SyncFlags {
    fn default() -> Self {
        Self {
            bookmarks: true,
            history: false,
            reading_list: false,
            cookies: false,
            passwords: false,
            extensions: false,
            history_days: Some(30),
            deduplicate: false,
            merge: false,
            verbose: false,
        }
    }
}

impl SyncFlags {
    /// 验证标志配置的安全性
    pub fn validate(&self) -> Result<()> {
        // 1. 严格禁止密码同步
        if self.passwords {
            return Err(anyhow!("❌ Error: Password export is blocked for security reasons. This tool will NEVER support password extraction."));
        }

        // 2. 严格禁止扩展程序同步 (技术上不可行且有风险)
        if self.extensions {
            return Err(anyhow!("❌ Error: Extension sync is not supported. Extensions store complex local state that cannot be safely transferred."));
        }

        // 3. 针对 Cookie 的警告
        if self.cookies {
            warn!("⚠️  WARNING: Exporting cookies affects active browser sessions.");
            warn!("   Importing these cookies elsewhere may overwrite existing sessions.");
            warn!("   Handle the exported file with extreme care as it contains session tokens!");
        }

        // 4. 检查是否至少选择了一种数据类型
        if !self.bookmarks && !self.history && !self.reading_list && !self.cookies {
            return Err(anyhow!("❌ Error: No data types selected. Please specify at least one of: --bookmarks, --history, --reading-list, --cookies"));
        }

        Ok(())
    }

    /// 获取启用的数据类型描述
    pub fn description(&self) -> String {
        let mut types = Vec::new();

        if self.bookmarks {
            types.push("Bookmarks");
        }
        if self.history {
            if let Some(days) = self.history_days {
                types.push(format!("History ({} days)", days).leak());
            } else {
                types.push("History (all)");
            }
        }
        if self.reading_list {
            types.push("Reading List");
        }
        if self.cookies {
            types.push("Cookies (⚠️)");
        }

        types.join(", ")
    }
}
