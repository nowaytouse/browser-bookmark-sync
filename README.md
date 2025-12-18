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
- **Flat Export** - Remove browser root folders (prevents nested "Imported > Waterfox > Brave")
- **Folder Filter** - Export only specific folders (e.g., "Temp" or "临时")
- **Smart Organize** - Auto-classify bookmarks using 48+ built-in rules
- **Empty Folder Cleanup** - Remove empty folders during export
- **Incremental Update** - Merge new bookmarks into existing export file
- **Unicode/Emoji Support** - Preserve folder names with emoji and special characters

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
| `check` | Check bookmark URL validity (dual-network) |
| `analyze` | Check for duplicates, empty folders, issues |
| `organize` | Smart organize by URL patterns (48+ rules) |
| `validate` | Validate bookmark integrity |
| `history` | Sync history between browsers |
| `backup` | Create full backup of all browser data |
| `rules` | Show available classification rules |
| `export-data` | Export sensitive data (passwords, cookies) |

## URL Validity Check (NEW)

Check if bookmark URLs are still valid using dual-network validation (proxy + direct).

```bash
# Check all bookmarks (direct connection only)
bsync check

# Check with proxy for geo-restricted URLs
bsync check --proxy http://127.0.0.1:7890

# Preview invalid bookmarks without deleting
bsync check --dry-run --verbose

# Delete confirmed invalid bookmarks
bsync check --proxy http://127.0.0.1:7890 --delete

# Check specific browser
bsync check -b safari --verbose
```

### Check Options

```bash
bsync check [OPTIONS]

-p, --proxy <URL>        Proxy server URL (e.g., http://127.0.0.1:7890)
-t, --timeout <SECS>     Request timeout (default: 10)
-c, --concurrency <N>    Concurrent requests (default: 10)
-b, --browsers <LIST>    Target browsers (default: all)
--delete                 Delete confirmed invalid bookmarks
--dry-run                Preview mode, no actual changes
-v, --verbose            Show HTTP status codes
```

### Validation Logic

- **Valid**: Either proxy OR direct connection succeeds (HTTP 2xx/3xx)
- **Invalid**: Both proxy AND direct connections fail (HTTP 4xx/5xx or error)
- **Uncertain**: Single network mode failure, timeout, or conflicting results

## Export Options

```bash
bsync export [OPTIONS]

-o, --output <FILE>      Output path (default: ~/Desktop/bookmarks.html)
-b, --browsers <LIST>    Source browsers (comma-separated, or 'all')
-d, --deduplicate        Remove duplicate bookmarks
-m, --merge              Flatten into single structure
--flat                   Remove browser root folders (Waterfox, Brave, etc.)
-r, --reading-list       Include Safari reading list
-f, --folder <NAME>      Only export specific folder
--history                Include browsing history
--history-days <N>       Days of history (default: 30, 0 = all)
--cookies                Include cookies
--clean                  Remove empty folders
--include <FILE>         Import from existing HTML file
-u, --update <FILE>      Incremental update: merge into existing file
-v, --verbose            Verbose output
```

### Flat Export (NEW)

Prevents nested folder structure when importing to browsers:

```bash
# Without --flat: Imported > Waterfox > Brave Nightly > ...
# With --flat: Your folders appear directly at top level

bsync export --flat -d --clean -o bookmarks.html
```

### Incremental Update (NEW)

Add new bookmarks to existing export without duplicates:

```bash
# First export
bsync export -o bookmarks.html

# Later: add only new bookmarks
bsync export -u bookmarks.html -o bookmarks.html
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
- **扁平导出** - 移除浏览器根文件夹（避免 "Imported > Waterfox > Brave" 嵌套）
- **文件夹过滤** - 仅导出特定文件夹（如"临时"）
- **智能整理** - 使用 48+ 内置规则自动分类书签
- **清理空文件夹** - 导出时移除空文件夹
- **增量更新** - 将新书签合并到现有导出文件
- **Unicode/Emoji 支持** - 保留带 emoji 和特殊字符的文件夹名称

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
| `check` | 检查收藏夹URL有效性（双网络验证） |
| `analyze` | 检查重复、空文件夹等问题 |
| `organize` | 按 URL 模式智能整理（48+ 规则） |
| `validate` | 验证书签完整性 |
| `history` | 在浏览器间同步历史记录 |
| `backup` | 创建所有浏览器数据的完整备份 |
| `rules` | 显示可用的分类规则 |
| `export-data` | 导出敏感数据（密码、cookies） |

## URL有效性检查（新功能）

使用代理+直连双网络验证检查收藏夹URL是否有效。

```bash
# 检查所有收藏夹（仅直连）
bsync check

# 使用代理检查（适用于地域限制的URL）
bsync check --proxy http://127.0.0.1:7890

# 预览无效收藏夹（不删除）
bsync check --dry-run --verbose

# 删除确认无效的收藏夹
bsync check --proxy http://127.0.0.1:7890 --delete
```

### 验证逻辑

- **有效**: 代理或直连任一成功（HTTP 2xx/3xx）
- **无效**: 代理和直连都失败（HTTP 4xx/5xx或连接错误）
- **不确定**: 单网络模式失败、超时或结果冲突

---

MIT License
