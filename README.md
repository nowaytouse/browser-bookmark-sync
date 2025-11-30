# 🔄 Browser Bookmark Sync

跨浏览器书签、历史记录、阅读列表同步工具。采用**中枢浏览器架构**，避免数据重复和混乱。

## ✨ 特性

- 🎯 **中枢浏览器模式** - 指定主力浏览器，其他浏览器自动清理
- 📚 **书签同步** - 保留完整文件夹结构，无扁平化
- 📜 **历史记录同步** - 跨浏览器合并浏览历史
- 📖 **阅读列表同步** - Safari 阅读列表迁移到中枢浏览器
- 🍪 **Cookies 同步** - 跨浏览器 Cookie 迁移
- ⏰ **定时同步** - 支持 Cron 表达式自动同步
- 🔒 **安全备份** - 每次操作前自动备份

## 🖥️ 支持的浏览器

| 浏览器 | 书签 | 历史 | 阅读列表 | Cookies |
|--------|------|------|----------|---------|
| **Brave Nightly** | ✅ | ✅ | - | ✅ |
| **Waterfox** | ✅ | ✅ | - | ✅ |
| **Brave** | ✅ | ✅ | - | ✅ |
| **Chrome** | ✅ | ✅ | - | ✅ |
| **Safari** | ✅ | ✅ | ✅ | - |
| **Firefox Nightly** | ✅ | ✅ | - | ✅ |
| **LibreWolf** | ✅ | ✅ | - | ✅ |

## 🚀 快速开始

### 一键同步（推荐）

双击运行 `sync-now.command`：

```bash
# 或在终端运行
./sync-now.command
```

这将自动：
1. 同步 Brave Nightly ↔ Waterfox 书签和历史
2. 提取 Safari 阅读列表到中枢浏览器
3. 清理非中枢浏览器的重复数据

### 命令行使用

```bash
# 查看所有检测到的浏览器
browser-bookmark-sync list

# 验证书签完整性
browser-bookmark-sync validate

# 设置中枢浏览器并同步
browser-bookmark-sync set-hubs \
  --browsers "waterfox,brave-nightly" \
  --sync-history \
  --sync-reading-list \
  --clear-others

# 仅同步书签（所有浏览器）
browser-bookmark-sync sync

# 同步历史记录（最近30天）
browser-bookmark-sync sync-history --days 30

# 定时同步（每30分钟）
browser-bookmark-sync schedule --cron "0 */30 * * * *"
```

## 📐 同步策略

### 中枢浏览器架构

```
┌─────────────────────────────────────────────────────┐
│                    中枢浏览器                        │
│         Waterfox  ←→  Brave Nightly                │
│         (完整数据)     (完整数据)                    │
└─────────────────────────────────────────────────────┘
                         ↑
                    数据迁移后清空
                         ↑
┌─────────────────────────────────────────────────────┐
│                   非中枢浏览器                       │
│     Chrome | Brave | Safari | LibreWolf            │
│     (清空)   (清空)  (清空)    (清空)               │
└─────────────────────────────────────────────────────┘
```

### 同步规则

1. **书签同步**
   - 选择文件夹结构最完整的浏览器作为基准
   - 保留完整的树形结构（无扁平化）
   - URL 去重（相同 URL 只保留一份）

2. **历史记录同步**
   - 合并所有浏览器的历史记录
   - 按 URL 去重
   - 按最后访问时间排序

3. **阅读列表同步**
   - Safari 阅读列表 → 中枢浏览器书签文件夹
   - 迁移后清空 Safari 阅读列表

4. **Profile 处理**
   - 仅同步 Default Profile
   - 其他 Profile 的重复数据会被清理

## 📁 文件结构

```
browser-sync/
├── src/
│   ├── main.rs          # CLI 入口
│   ├── browsers.rs      # 浏览器适配器
│   ├── sync.rs          # 同步引擎
│   ├── scheduler.rs     # 定时任务
│   └── validator.rs     # 数据验证
├── sync-now.command     # 一键同步脚本 (macOS)
├── empty_bookmarks.json # 空书签模板
└── README.md
```

## 🔧 编译安装

```bash
# 克隆仓库
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync

# 编译
cargo build --release

# 安装到系统（可选）
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## ⚠️ 注意事项

1. **关闭浏览器** - 同步前请关闭所有浏览器，避免数据库锁定
2. **自动备份** - 每次同步前会自动创建备份到 `~/Desktop/browser_backup_*`
3. **Safari 权限** - 首次运行需要授予完全磁盘访问权限

## 📊 数据统计示例

```
📊 Hub Configuration Complete!
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Hub browsers: ["waterfox", "brave-nightly"]
  Bookmarks: 24217 URLs, 1250 folders
  History: 20256 items synced
  Reading list: 136 items synced
  Non-hub browsers: CLEARED
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## 📜 License

MIT License
