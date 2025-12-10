# Browser Bookmark Sync (bsync)

Cross-browser bookmark management tool for macOS. Export, merge, deduplicate, and organize bookmarks across multiple browsers.

[English](#features) | [中文](#功能特性)

---

## Features

### Multi-Browser Support
- Safari, Chrome, Brave, Edge, Arc, Waterfox, Firefox
- Auto-detection of installed browsers
- Reading list export (Safari)

### Export Capabilities
- **Bookmarks** - Standard Netscape HTML format
- **History** - Browsing history with configurable date range
- **Cookies** - Export to CSV, JSON, or Netscape format
- **Passwords** - Chromium-based browsers (requires keychain access)
- **Downloads** - Download history

### Smart Processing
- **Deduplication** - Remove duplicate bookmarks by URL
- **Merge** - Flatten folder structure from multiple browsers
- **Folder Filter** - Export only specific folders (e.g., "Temp" or "临时")
- **Smart Organize** - Auto-classify bookmarks using 48+ built-in rules
- **Empty Folder Cleanup** - Remove empty folders during export

### Safety Features
- **Non-destructive by default** - Export only, no modifications
- **Backup command** - Full browser data backup
- **Dry-run mode** - Preview changes before applying
- **Database safety** - Copies databases before reading

## Quick Start

```bash
# Build
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync

# List detected browsers
bsync list

# Export all bookmarks (deduplicated, merged)
bsync export -d --merge

# Export Safari with reading list
bsync export -b safari --reading-list

# Export specific folder only
bsync export -f "临时" -d

# Export with history (last 30 days)
bsync export --history --history-days 30

# Analyze bookmarks for issues
bsync analyze

# Smart organize (preview)
bsync organize --dry-run
```

## Commands

| Command | Description |
|---------|-------------|
| `list` | List detected browsers and bookmark counts |
| `export` | Export bookmarks/history/cookies to file |
| `analyze` | Check for duplicates, empty folders, issues |
| `organize` | Smart organize by URL patterns (48+ rules) |
| `validate` | Validate bookmark integrity |
| `history` | Sync history between browsers |
| `backup` | Create full backup of all browser data |
| `rules` | Show available classification rules |
| `export-data` | Export sensitive data (passwords, cookies) |

## Export Options

```bash
bsync export [OPTIONS]

-o, --output <FILE>      Output path (default: ~/Desktop/bookmarks.html)
-b, --browsers <LIST>    Source browsers (comma-separated, or 'all')
-d, --deduplicate        Remove duplicate bookmarks
-m, --merge              Flatten into single structure
-r, --reading-list       Include Safari reading list
-f, --folder <NAME>      Only export specific folder
--history                Include browsing history
--history-days <N>       Days of history (default: 30, 0 = all)
--cookies                Include cookies
--clean                  Remove empty folders
--include <FILE>         Import from existing HTML file
-v, --verbose            Verbose output
```

## Dependencies

```bash
# macOS only (uses native browser database formats)
cargo build --release
```

---

## 功能特性

### 多浏览器支持
- Safari、Chrome、Brave、Edge、Arc、Waterfox、Firefox
- 自动检测已安装的浏览器
- Safari 阅读列表导出

### 导出功能
- **书签** - 标准 Netscape HTML 格式
- **历史记录** - 可配置日期范围
- **Cookies** - 导出为 CSV、JSON 或 Netscape 格式
- **密码** - Chromium 系浏览器（需要钥匙串访问权限）
- **下载记录** - 下载历史

### 智能处理
- **去重** - 按 URL 去除重复书签
- **合并** - 将多个浏览器的文件夹结构扁平化
- **文件夹过滤** - 仅导出特定文件夹（如"临时"）
- **智能整理** - 使用 48+ 内置规则自动分类书签
- **清理空文件夹** - 导出时移除空文件夹

### 安全特性
- **默认非破坏性** - 仅导出，不修改
- **备份命令** - 完整浏览器数据备份
- **预览模式** - 应用更改前预览
- **数据库安全** - 读取前复制数据库

## 快速开始

```bash
# 编译
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync

# 列出检测到的浏览器
bsync list

# 导出所有书签（去重、合并）
bsync export -d --merge

# 导出 Safari 及阅读列表
bsync export -b safari --reading-list

# 仅导出特定文件夹
bsync export -f "临时" -d

# 导出含历史记录（最近30天）
bsync export --history --history-days 30

# 分析书签问题
bsync analyze

# 智能整理（预览）
bsync organize --dry-run
```

## 命令说明

| 命令 | 说明 |
|------|------|
| `list` | 列出检测到的浏览器和书签数量 |
| `export` | 导出书签/历史/cookies 到文件 |
| `analyze` | 检查重复、空文件夹等问题 |
| `organize` | 按 URL 模式智能整理（48+ 规则） |
| `validate` | 验证书签完整性 |
| `history` | 在浏览器间同步历史记录 |
| `backup` | 创建所有浏览器数据的完整备份 |
| `rules` | 显示可用的分类规则 |
| `export-data` | 导出敏感数据（密码、cookies） |

---

MIT License
