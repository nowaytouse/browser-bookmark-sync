# 🔄 Browser Bookmark Sync

A powerful cross-browser synchronization tool for bookmarks, history, and cookies. Features an intelligent **Rule Engine** for automatic bookmark classification and a **Hub Browser Architecture** for organized data management.

[中文文档](./README_CN.md)

## ✨ Core Features

### 🧠 Smart Organization (Rule Engine)
- **18 Built-in Classification Rules** - Automatically categorize bookmarks by URL patterns
- **Custom Rules Support** - Load your own rules from JSON files
- **Multi-dimensional Matching** - URL, domain, path, and title pattern matching
- **Priority-based Processing** - Higher priority rules match first

### 🎯 Hub Browser Architecture
- **Designate Primary Browsers** - Sync between hub browsers, optionally clean others
- **Full Data Sync** - Bookmarks, history, reading lists, and cookies in one command
- **Preserves Structure** - Complete folder hierarchy maintained, no flattening

### 🔄 Data Management
- **Global Deduplication** - Smart removal of duplicate URLs across entire bookmark tree
- **Empty Folder Cleanup** - Automatic removal of empty bookmark folders
- **Safe Backups** - Automatic backup before every operation

## 🖥️ Supported Browsers

| Browser | Bookmarks | History | Cookies | Reading List |
|---------|-----------|---------|---------|--------------|
| **Waterfox** | ✅ | ✅ | ✅ | - |
| **Brave Nightly** | ✅ | ✅ | ✅ | - |
| **Brave** | ✅ | ✅ | ✅ | - |
| **Chrome** | ✅ | ✅ | ✅ | - |
| **Safari** | ✅ | ✅ | - | ✅ |
| **Firefox Nightly** | ✅ | ✅ | ✅ | - |

## 🚀 Quick Start

### Basic Sync

```bash
# Full sync between hub browsers (bookmarks + history + cookies)
browser-bookmark-sync sync

# Preview changes without executing
browser-bookmark-sync sync --dry-run

# Custom hub browsers
browser-bookmark-sync sync --browsers "chrome,brave"
```

### Smart Organization

```bash
# Automatically classify all bookmarks using rule engine
browser-bookmark-sync smart-organize

# Preview classification results
browser-bookmark-sync smart-organize --dry-run --show-stats

# Only organize uncategorized bookmarks (not in folders)
browser-bookmark-sync smart-organize --uncategorized-only

# Use custom rules
browser-bookmark-sync smart-organize --rules-file my-rules.json

# View all available rules
browser-bookmark-sync list-rules
```

### Cleanup & Maintenance

```bash
# Remove duplicate bookmarks
browser-bookmark-sync cleanup --remove-duplicates

# Remove empty folders
browser-bookmark-sync cleanup --remove-empty-folders

# Full cleanup
browser-bookmark-sync cleanup --remove-duplicates --remove-empty-folders
```

## 🧠 Rule Engine

The intelligent classification engine automatically organizes bookmarks into categories based on URL patterns, domains, paths, and titles.

### Built-in Categories

| Priority | Category | Folder Name | Description |
|----------|----------|-------------|-------------|
| 100 | **Login** | 登录入口 | Login pages, SSO, OAuth endpoints |
| 90 | **Social** | 社交媒体 | Twitter, Facebook, Instagram, etc. |
| 85 | **Video** | 视频流媒体 | YouTube, Netflix, Bilibili, etc. |
| 80 | **Dev** | 开发工具 | GitHub, StackOverflow, npm, etc. |
| 75 | **Shopping** | 购物网站 | Amazon, Taobao, eBay, etc. |
| 70 | **News** | 新闻资讯 | CNN, BBC, Reuters, etc. |
| 65 | **Docs** | 文档参考 | Wikipedia, ReadTheDocs, etc. |
| 60 | **Cloud** | 云存储 | Google Drive, Dropbox, etc. |
| 55 | **Email** | 邮箱通讯 | Gmail, Outlook, etc. |
| 50 | **Finance** | 金融理财 | PayPal, banks, investment sites |
| 45 | **AI** | AI工具 | ChatGPT, Claude, Midjourney, etc. |
| 40 | **Design** | 设计创意 | Figma, Canva, Dribbble, etc. |
| 35 | **Education** | 教育学习 | Coursera, Udemy, etc. |
| 30 | **Music** | 音乐音频 | Spotify, Apple Music, etc. |
| 25 | **Gaming** | 游戏娱乐 | Steam, Epic Games, etc. |
| 20 | **Forum** | 论坛社区 | Reddit, Quora, V2EX, etc. |
| 15 | **Admin** | 管理后台 | Admin panels, dashboards |
| 10 | **API** | API服务 | API endpoints, web services |

### Custom Rules

Create a JSON file with custom rules:

```json
[
  {
    "name": "work-tools",
    "folder_name": "工作工具",
    "folder_name_en": "Work Tools",
    "url_patterns": ["jira", "confluence", "slack"],
    "domain_patterns": ["atlassian.com", "slack.com"],
    "path_patterns": ["/projects", "/workspace"],
    "title_patterns": ["project", "工作"],
    "priority": 95,
    "description": "Work-related tools and platforms"
  }
]
```

Then use it:

```bash
browser-bookmark-sync smart-organize --rules-file work-rules.json
```

### Rule Matching Logic

Each rule can match bookmarks using four methods:

1. **URL Patterns** - Match anywhere in the full URL
   - Example: `login` matches `https://example.com/login`
   
2. **Domain Patterns** - Match in the domain portion
   - Example: `github.com` matches `https://github.com/user/repo`
   
3. **Path Patterns** - Match in the URL path
   - Example: `/admin` matches `https://example.com/admin/dashboard`
   
4. **Title Patterns** - Match in the bookmark title
   - Example: `文档` matches "API 文档"

Rules are processed by priority (highest first). First matching rule wins.

## 📐 Architecture

### Hub Browser Model

```
┌─────────────────────────────────────────────────────┐
│                   HUB BROWSERS                       │
│         Waterfox  ←──────→  Brave Nightly           │
│                                                      │
│   📚 Bookmarks    📜 History    🍪 Cookies          │
│   (Full Sync)     (Full Sync)   (Full Sync)         │
└─────────────────────────────────────────────────────┘
                         ↑
              Optional: --clear-others
                         ↑
┌─────────────────────────────────────────────────────┐
│                 NON-HUB BROWSERS                     │
│        Chrome | Brave | Safari | Firefox            │
│              (Data migrated then cleared)           │
└─────────────────────────────────────────────────────┘
```

### Smart Deduplication

The deduplication engine uses intelligent rules:

1. **Depth Priority** - Prefer bookmarks deeper in folder structure
2. **Recency Priority** - Among same depth, prefer newer bookmarks
3. **URL Normalization** - Trailing slashes and fragments removed for comparison

```
Before: https://example.com (root) + https://example.com (in Work folder)
After:  https://example.com (kept in Work folder only)
```

## 📊 Commands Reference

### Synchronization

| Command | Description |
|---------|-------------|
| `sync` | Full sync (bookmarks + history + cookies) between hub browsers |
| `sync --clear-others` | Full sync + clear non-hub browser data |
| `sync-history` | Sync ALL history only |
| `sync-cookies` | Sync cookies only |
| `sync-reading-list` | Sync reading lists |
| `sync-scenario` | Sync specific folder across browsers |
| `set-hubs` | Configure and sync hub browsers |

### Organization

| Command | Description |
|---------|-------------|
| `smart-organize` | **Auto-classify bookmarks using rule engine** |
| `smart-organize --show-stats` | Show classification statistics |
| `organize` | Move homepage bookmarks to dedicated folder |
| `list-rules` | Display all available classification rules |

### Maintenance

| Command | Description |
|---------|-------------|
| `cleanup --remove-duplicates` | Remove duplicate bookmarks |
| `cleanup --remove-empty-folders` | Remove empty bookmark folders |
| `validate` | Check data integrity across all browsers |
| `list` | Show detected browsers and paths |

### Options

```bash
# Common options for most commands
-b, --browsers <BROWSERS>    Target browsers (comma-separated)
-d, --dry-run                Preview without making changes
-v, --verbose                Detailed output

# Smart organize specific
-r, --rules-file <FILE>      Load custom rules from JSON file
    --uncategorized-only     Only organize root-level bookmarks
    --show-stats             Display classification statistics
```

## 📊 Test Results

```
Test Suite: 48 tests (40 unit + 8 integration) ✅

Sync Statistics:
├── Bookmarks: 41,661 URLs, 1,936 folders
├── History: 30,301 unique items
├── Cookies: 925 unique
├── Rule Engine: 18 built-in classification rules
└── Performance: ~1.1s (release build)
```

## 🔧 Installation

```bash
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync
cargo build --release

# Run tests
cargo test

# Install (optional)
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## ⚠️ Important Notes

1. **Close browsers before syncing** - Browsers will overwrite changes if running
2. **Backups are automatic** - Saved to `~/Desktop/browser_backup_*`
3. **Default hubs** - Waterfox + Brave Nightly (customizable with `--browsers`)
4. **Protected folders** - Existing category folders won't be re-organized

## 📁 Project Structure

```
browser-bookmark-sync/
├── src/
│   ├── main.rs          # CLI commands and entry point
│   ├── sync.rs          # Sync engine and rule engine
│   ├── browsers.rs      # Browser adapters (Chromium/Firefox/Safari)
│   ├── validator.rs     # Data validation
│   └── scheduler.rs     # Periodic sync scheduler
├── tests/
│   └── integration_test.rs
├── examples/
│   └── custom-rules.json
└── Cargo.toml
```

## 📜 License

MIT License
