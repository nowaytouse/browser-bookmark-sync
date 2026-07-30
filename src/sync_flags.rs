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

    /// 是否去重 (用于export命令)
    pub deduplicate: bool,

    /// 是否合并文件夹结构 (用于export命令)
    pub merge: bool,

    /// 详细输出 (用于export命令)
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
        // 1. 密码导出 - 允许但强烈警告
        if self.passwords {
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            warn!("🔴 CRITICAL SECURITY WARNING: Password Export Enabled");
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            warn!("⚠️  Browser passwords are ENCRYPTED with OS-level protection.");
            warn!("⚠️  Only encrypted blobs can be exported - NOT plaintext passwords.");
            warn!("⚠️  These encrypted passwords CANNOT be imported to other browsers.");
            warn!(
                "⚠️  For password migration, use browser's built-in export or a password manager."
            );
            warn!("");
            warn!("🔒 What you'll get: Encrypted password metadata (URLs, usernames, timestamps)");
            warn!("❌ What you WON'T get: Actual decrypted passwords");
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        }

        // 2. 扩展程序导出 - 允许但说明限制
        if self.extensions {
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            warn!("⚠️  EXTENSION EXPORT LIMITATIONS");
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            warn!("📦 Extensions contain complex local state that CANNOT be fully transferred:");
            warn!("   • Extension settings and preferences are browser-specific");
            warn!("   • Login states and tokens cannot be migrated");
            warn!("   • Some extensions are browser-exclusive (Chrome-only, Firefox-only)");
            warn!("");
            warn!("✅ What you'll get: Extension list with metadata (name, version, permissions)");
            warn!("❌ What you WON'T get: Extension data, settings, or automatic installation");
            warn!("💡 Recommendation: Use this list to manually reinstall extensions");
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        }

        // 3. 针对 Cookie 的警告
        if self.cookies {
            warn!("⚠️  WARNING: Exporting cookies affects active browser sessions.");
            warn!("   Importing these cookies elsewhere may overwrite existing sessions.");
            warn!("   Handle the exported file with extreme care as it contains session tokens!");
        }

        // 4. 检查是否至少选择了一种数据类型
        if !self.bookmarks
            && !self.history
            && !self.reading_list
            && !self.cookies
            && !self.passwords
            && !self.extensions
        {
            return Err(anyhow!("❌ Error: No data types selected. Please specify at least one of: --bookmarks, --history, --reading-list, --cookies, --passwords, --extensions"));
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
                types.push(format!("History ({days} days)").leak());
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
        if self.passwords {
            types.push("Passwords (🔴 ENCRYPTED ONLY)");
        }
        if self.extensions {
            types.push("Extensions (⚠️ METADATA ONLY)");
        }

        types.join(", ")
    }
}
