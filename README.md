# 🔖 Browser Bookmark Sync

A powerful cross-browser bookmark management tool for macOS. Merge, deduplicate, and export bookmarks from multiple browsers into a single HTML file.

## ✨ Features

- **🌐 Multi-Browser Support**: Safari, Chrome, Brave, Brave Nightly, Waterfox, Firefox
- **📤 HTML Export**: Export to standard Netscape HTML format (importable by all browsers)
- **🧹 Smart Deduplication**: Remove duplicate bookmarks across all sources
- **🧠 Auto-Classification**: 48 built-in rules to organize bookmarks by category
- **🔍 Anomaly Detection**: Detect bulk imports, history pollution, NSFW content
- **💾 Backup & Restore**: Full backup and restore capabilities

## 🚀 Quick Start

### Installation

```bash
# Clone and build
git clone https://github.com/user/browser-sync.git
cd browser-sync
cargo build --release

# Add to PATH (optional)
cp target/release/browser-bookmark-sync /usr/local/bin/
```

### Basic Usage

```bash
# List detected browsers
browser-bookmark-sync list

# Export all bookmarks to HTML (RECOMMENDED)
browser-bookmark-sync export-html -o ~/Desktop/my_bookmarks.html -d

# Export specific browsers with deduplication
browser-bookmark-sync export-html -b "safari,brave-nightly" -d --merge

# Smart organize bookmarks
browser-bookmark-sync smart-organize -b safari --dry-run --show-stats
```

## 📖 Commands

| Command | Description |
|---------|-------------|
| `list` | List all detected browsers and bookmark locations |
| `export-html` | Export bookmarks to HTML file (recommended) |
| `validate` | Validate bookmark integrity |
| `cleanup` | Remove duplicates and empty folders |
| `smart-organize` | Auto-classify bookmarks by URL patterns |
| `list-rules` | Show available classification rules |
| `sync-history` | Sync browsing history between hub browsers |
| `analyze` | Analyze bookmarks (NSFW detection) |
| `master-backup` | Create comprehensive backup |
| `restore-backup` | Restore from backup |
| `clear-bookmarks` | Clear browser bookmarks (debug only) |

## 📤 Export to HTML (Recommended Workflow)

The recommended way to manage bookmarks is to export them to HTML and manually import into your target browser. This avoids sync conflicts.

```bash
# Step 1: Export all bookmarks with deduplication
browser-bookmark-sync export-html \
  -b "safari,brave-nightly,waterfox" \
  -d --merge \
  -o ~/Desktop/all_bookmarks.html

# Step 2: Manually import the HTML file into your browser
# - Safari: File → Import From → Bookmarks HTML File
# - Chrome/Brave: Bookmarks → Import Bookmarks and Settings
# - Firefox: Bookmarks → Manage Bookmarks → Import and Backup
```

### Export Options

```bash
-o, --output <FILE>      Output HTML file path
-b, --browsers <LIST>    Source browsers (comma-separated, default: all)
-d, --deduplicate        Remove duplicate bookmarks
    --merge              Merge into flat structure (no browser folders)
    --clean-empty        Remove empty folders before export
    --include-html <FILE> Also import from existing HTML backup
    --clear-after        Clear bookmarks from source browsers after export
-v, --verbose            Show detailed output
```

### Clear After Export

The `--clear-after` option will delete all bookmarks from source browsers after successful export:

```bash
# Export and clear source bookmarks
browser-bookmark-sync export-html -d --merge --clear-after
```

⚠️ **WARNING**: If browser sync is enabled (Firefox Sync, Chrome Sync, iCloud, etc.), deletion may be ineffective or cause unpredictable bookmark versions. Consider disabling sync before using this option.

## 🧠 Smart Organization

Automatically classify bookmarks into 48 categories:

```bash
# Preview classification (dry-run)
browser-bookmark-sync smart-organize -b safari --dry-run --show-stats

# Apply classification
browser-bookmark-sync smart-organize -b safari

# Use custom rules
browser-bookmark-sync smart-organize -r custom-rules.json
```

### Built-in Categories

- 🎬 Streaming Sites, Video Platforms
- 🎮 Gaming, Game Stores
- 💻 Development, GitHub, Stack Overflow
- 📱 Social Media, Forums
- 🛒 Shopping, E-commerce
- 📰 News, Blogs
- 🎨 Design, Creative Tools
- And 40+ more...

## 🔄 History Sync

Sync browsing history between hub browsers:

```bash
# Sync last 30 days of history
browser-bookmark-sync sync-history -b "waterfox,brave-nightly"

# Sync last 7 days
browser-bookmark-sync sync-history -b "waterfox,brave-nightly" -d 7

# Preview mode
browser-bookmark-sync sync-history --dry-run
```

## 🔍 Bookmark Analysis

Analyze bookmarks for duplicates and NSFW content:

```bash
browser-bookmark-sync analyze -b safari
```

Detects:
- **Duplicate URLs**: Same URL bookmarked multiple times
- **Empty Folders**: Folders with no bookmarks
- **NSFW Content**: Adult content statistics (info only)

## 💾 Backup & Restore

```bash
# Create master backup
browser-bookmark-sync master-backup -o ~/Desktop/BookmarkBackup

# Restore from backup
browser-bookmark-sync restore-backup -b waterfox -f backup.sqlite
```

## 🌐 Supported Browsers

| Browser | Bookmarks | History | Cookies |
|---------|-----------|---------|---------|
| Safari | ✅ | ✅ | ❌ |
| Chrome | ✅ | ✅ | ✅ |
| Brave | ✅ | ✅ | ✅ |
| Brave Nightly | ✅ | ✅ | ✅ |
| Waterfox | ✅ | ✅ | ✅ |
| Firefox | ✅ | ✅ | ✅ |

## ⚠️ Important Notes

1. **Close browsers before operations**: Some browsers lock their database files
2. **Use HTML export**: Avoid direct browser writes to prevent sync conflicts
3. **Backup first**: Always create backups before major operations
4. **Manual import**: Import HTML files manually for best results

## 📊 Example Output

```
📤 导出书签到HTML文件
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📄 输出: ~/Desktop/bookmarks.html
🌐 来源: safari,brave-nightly
🔀 合并模式
🧹 去重复
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ✅ Safari : 136054 书签
  ✅ Brave Nightly : 42272 书签
📊 收集完成: 178326 书签
  ✅ 移除 154805 重复书签
✅ 导出完成!
   📄 文件: ~/Desktop/bookmarks.html
   📊 书签数: 23521

🎉 导出完成! 23521 书签
💡 请手动导入到目标浏览器，避免被同步覆盖
```

## 🛠️ Development

```bash
# Run tests
cargo test

# Build release
cargo build --release

# Run with debug logging
RUST_LOG=debug browser-bookmark-sync list
```

## 📄 License

MIT License

## 🤝 Contributing

Contributions welcome! Please read the contributing guidelines first.
