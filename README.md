# 🔄 Browser Bookmark Sync

A powerful cross-browser synchronization tool for bookmarks, history, and cookies. Features an intelligent **Rule Engine** for automatic bookmark classification and a **Hub Browser Architecture** for organized data management.

[中文文档](./README_CN.md)

## ✨ Core Features

### 🔄 Intelligent Sync Modes
- **Incremental Sync** - Only sync changes since last sync (fast, efficient)
- **Full Sync** - Complete synchronization of all data (thorough)
- **Multi-stage Deduplication** - Pre-merge, post-merge, and validation stages
- **Comprehensive Validation** - Pre-sync and post-sync integrity checks

### 🧠 Smart Organization (Rule Engine)
- **75 Built-in Classification Rules** - Automatically categorize bookmarks by URL patterns
- **Custom Rules Support** - Load your own rules from JSON files
- **Multi-dimensional Matching** - URL, domain, path, and title pattern matching
- **Priority-based Processing** - Higher priority rules match first
- **Re-classification Support** - Automatically re-classify "Uncategorized" bookmarks

### 🎯 Hub Browser Architecture
- **Designate Primary Browsers** - Sync between hub browsers, optionally clean others
- **Full Data Sync** - Bookmarks, history, reading lists, and cookies in one command
- **Preserves Structure** - Complete folder hierarchy maintained, no flattening

### 🔄 Data Management
- **Global Deduplication** - Smart removal of duplicate URLs across entire bookmark tree
- **Empty Folder Cleanup** - Automatic removal of empty bookmark folders (99.9% reduction achieved)
- **Folder Structure Deduplication** - Remove duplicate folder hierarchies
- **Invalid Entry Removal** - Clean up folders with "/" or empty names
- **Safe Backups** - Automatic backup before every operation
- **Sync Statistics** - Detailed reports on synced items, duplicates removed, errors

## 🖥️ Supported Browsers

| Browser | Bookmarks | History | Cookies | Reading List |
|---------|-----------|---------|---------|--------------| | **Waterfox** | ✅ | ✅ | ✅ | - |
| **Brave Nightly** | ✅ | ✅ | ✅ | - |
| **Brave** | ✅ | ✅ | ✅ | - |
| **Chrome** | ✅ | ✅ | ✅ | - |
| **Safari** | ✅ | ✅ | - | ✅ |
| **Firefox Nightly** | ✅ | ✅ | ✅ | - |

## 🚀 Quick Start

### Basic Sync

```bash
# Incremental sync (default) - only sync changes since last sync
browser-bookmark-sync sync --mode incremental

# Full sync - sync all bookmarks
browser-bookmark-sync sync --mode full

# Preview changes without executing
browser-bookmark-sync sync --dry-run

# Custom hub browsers
browser-bookmark-sync sync --browsers "chrome,brave"

# Validate bookmark integrity
browser-bookmark-sync validate --detailed

# List detected browsers
browser-bookmark-sync list
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

### Built-in Categories (75 Rules)

_Note: 27 new rules added (marked with 🆕) to minimize uncategorized bookmarks_

#### Core Rules (1-48)

| Priority | Category | Folder Name | Description |
|----------|----------|-------------|-------------|
| 100 | **Login** | 登录入口 | Login pages, SSO, OAuth endpoints |
| 95 | **NSFW** | NSFW内容 | Adult content (auto-detected) |
| 90 | **Social** | 社交媒体 | Twitter, Facebook, Instagram, etc. |
| 88 | **Discord** | Discord社群 | Discord servers and invites |
| 85 | **Video** | 视频流媒体 | YouTube, Netflix, Bilibili, etc. |
| 80 | **Dev** | 开发工具 | GitHub, StackOverflow, npm, etc. |
| 76 | 🆕 **DevOps** | DevOps运维 | Jenkins, GitLab CI, CircleCI, etc. |
| 75 | **Shopping** | 购物网站 | Amazon, Taobao, eBay, etc. |
| 74 | 🆕 **Database** | 数据库服务 | PostgreSQL, MongoDB, Redis, etc. |
| 72 | **Anime** | 动漫二次元 | MyAnimeList, Anilist, manga sites |
| 70 | **News** | 新闻资讯 | CNN, BBC, Reuters, etc. |
| 68 | **Streaming** | 直播平台 | Twitch, live streaming platforms |
| 66 | 🆕 **Containers** | 容器云原生 | Docker, Kubernetes, K8s, etc. |
| 65 | **Docs** | 文档参考 | Wikipedia, ReadTheDocs, etc. |
| 62 | 🆕 **API Tools** | API工具 | Postman, Swagger, Insomnia, etc. |
| 60 | **Cloud** | 云存储 | Google Drive, Dropbox, etc. |
| 58 | 🆕 **Monitoring** | 服务器监控 | Grafana, Prometheus, Datadog, etc. |
| 56 | **DevTools** | 开发者工具 | JetBrains, VS Code, IDEs |
| 55 | **Email** | 邮箱通讯 | Gmail, Outlook, etc. |
| 54 | 🆕 **Blockchain** | 区块链加密 | Ethereum, Bitcoin, NFT, DeFi, etc. |
| 53 | 🆕 **Maps** | 地图导航 | Google Maps, Amap, etc. |
| 52 | **ImageHost** | 图床托管 | Imgur, ibb.co, image hosting |
| 51 | 🆕 **JP/KR** | 日韩服务 | Japanese & Korean platforms |
| 50 | **Finance** | 金融理财 | PayPal, banks, investment sites |
| 49 | 🆕 **Translation** | 翻译服务 | Google Translate, DeepL, etc. |
| 48 | **Directories** | 导航目录 | Link aggregators, directories |
| 47 | 🆕 **Health** | 健康医疗 | WebMD, Mayo Clinic, etc. |
| 46 | **Chinese** | 中文平台 | Baidu, Zhihu, Bilibili, etc. |
| 45 | **AI** | AI工具 | ChatGPT, Claude, Midjourney, etc. |
| 44 | **Creative** | 设计素材 | Adobe, icons, fonts, design |
| 43 | 🆕 **Jobs** | 求职招聘 | LinkedIn, Indeed, BOSS直聘, etc. |
| 42 | **Security** | 安全隐私 | VPN, privacy tools, antivirus |
| 41 | 🆕 **Travel** | 旅游出行 | Booking, Airbnb, Ctrip, etc. |
| 40 | **Hardware** | 硬件技术 | NVIDIA, AMD, tech reviews |
| 39 | 🆕 **Food** | 外卖美食 | UberEats, Meituan, Ele.me, etc. |
| 38 | **Linux** | Linux开源 | Arch, Ubuntu, open source |
| 37 | 🆕 **Podcast** | 播客节目 | Apple Podcasts, Spotify, etc. |

#### Extended Rules (49-75)

| Priority | Category | Folder Name | Description |
|----------|----------|-------------|-------------|
| 36 | **Microsoft** | 微软服务 | Microsoft products and services |
| 34 | **Apple** | 苹果服务 | Apple products and services |
| 33 | 🆕 **Licensing** | 开源许可 | Open source licenses |
| 32 | **Google** | 谷歌服务 | Google products and services |
| 31 | 🆕 **Weather** | 天气服务 | Weather forecast services |
| 30 | **Music** | 音乐音频 | Spotify, Apple Music, etc. |
| 29 | 🆕 **E-books** | 电子书阅读 | Kindle, Goodreads, Z-Library, etc. |
| 28 | **Torrents** | 下载资源 | Torrent sites, downloads |
| 27 | 🆕 **Comics** | 漫画在线 | Webtoons, online comics |
| 25 | 🆕 **Fonts** | 字体资源 | Google Fonts, font downloads |
| 25 | **Gaming** | 游戏娱乐 | Steam, Epic Games, etc. |
| 23 | 🆕 **Photography** | 摄影图片 | 500px, Flickr, photo platforms |
| 22 | **Extensions** | 浏览器扩展 | Browser extensions, themes |
| 21 | 🆕 **Sports** | 体育运动 | ESPN, NBA, sports events |
| 20 | **Forum** | 论坛社区 | Reddit, Quora, V2EX, etc. |
| 19 | 🆕 **Secondhand** | 二手交易 | eBay, Xianyu, marketplaces |
| 18 | **Tools** | 在线工具 | Online utilities, converters |
| 17 | 🆕 **Deals** | 团购优惠 | Groupon, SMZDM, etc. |
| 16 | **Productivity** | 效率工具 | Notion, Trello, note-taking |
| 14 | **GameCommunity** | 游戏社区 | Steam community, mods, wikis |
| 13 | 🆕 **Price Tracking** | 价格比较 | Price comparison platforms |
| 12 | 🆕 **URL Shorteners** | 短链接服务 | bit.ly, short links |
| 11 | 🆕 **Localhost** | 本地开发 | localhost, local servers |
| 10 | **Blogs** | 博客站点 | WordPress, Medium, blogs |
| 8 | **Hosting** | 托管项目 | GitHub Pages, Vercel, Netlify |

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

### Sync Strategy (Important!)

**This is NOT incremental sync.** The synchronization uses a "Best Structure Wins" strategy:

#### Bookmarks Sync Logic

```
Phase 1: Read all bookmarks from ALL browsers
         ↓
Phase 2: Score each browser:
         Score = (folder_count × 1000) + url_count
         (Folder structure is prioritized)
         ↓
Phase 3: Select browser with HIGHEST score as BASE
         ↓
Phase 4: Global deduplication on base bookmarks
         ↓
Phase 5: Write merged result to ALL hub browsers
```

**Example:**
```
Before sync:
  Waterfox:     66,023 URLs, 3,188 folders → Score: 3,254,023 ✓ (selected as base)
  Brave Nightly: 53,658 URLs, 1,904 folders → Score: 1,957,658

After sync:
  Both browsers: 23,514 URLs (after dedup), 3,188 folders
```

#### History Sync Logic
- **Merge all history** from all browsers
- **Deduplicate by URL** (keep first occurrence)
- Write to all hub browsers

#### Cookies Sync Logic
- **Merge all cookies** from all browsers  
- **Deduplicate by host+name+path**
- Write to all hub browsers

> ⚠️ **Warning**: This is OVERWRITE sync, not merge sync. The browser with best folder structure becomes the source of truth. Other browsers' unique bookmarks NOT in this structure will be lost.

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

Real-world Sync Test (Waterfox ↔ Brave Nightly):
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Before:
    Waterfox:      66,023 URLs, 3,188 folders
    Brave Nightly: 53,658 URLs, 1,904 folders
  
  After (both browsers):
    Bookmarks: 23,514 URLs, 3,188 folders
    History:   39,287 items (merged & deduped)
    Cookies:   952 items
  
  Performance: ~1.5s (release build)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
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
