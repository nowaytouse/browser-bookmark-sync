# Browser Bookmark Sync (bsync)

> A high-performance, cross-browser bookmark synchronization, deduplication, dead-link checking, and export toolkit for macOS built in Rust — now featuring a modern Web GUI!

[English](#features) | [简体中文](#功能特性)

---

## Key Features

### 🌐 Modern Web GUI Dashboard (`bsync gui`)
- **Visual Control Panel**: Launch a local dark-mode glassmorphism Web GUI with `bsync gui` or `bsync ui`.
- **Browser Auto-Detection**: Live scan of installed browsers (Safari, Chrome, Brave, Waterfox, LibreWolf, Firefox Nightly) and exact bookmark counts.
- **One-Click Export & Merge**: Visually configure export rules, deduplication, and merge operations.
- **Real-Time Log Stream**: Built-in Server-Sent Events (SSE) log terminal tracking live engine activity.

### 📚 Multi-Browser & All-Profile Support
- **Supported Browsers**: Safari, Chrome, Brave, Edge, Arc, Waterfox, Firefox, LibreWolf.
- **All User Profiles**: Scans all user profiles across Chromium and Gecko browser directories to merge hidden or multi-account bookmarks into one file.
- **Safari Special Support**: Export Reading Lists directly as accessible HTML bookmarks.

### ⚡ Smart Processing & Exporting
- **URL Deduplication**: Automatically eliminate duplicate links by canonical URL matching.
- **Flat Structure Export**: Strip unnecessary root directory nesting (e.g. `Imported > Waterfox > Brave`) to keep top-level folders clean.
- **Smart Categorization**: Classify bookmarks with local Ollama AI models (`qwen2.5:7b`), with built-in rule engines as fallback.
- **Folder Filtering**: Export targeted sub-folders (e.g. `Temp` or `临时`).
- **Unicode & Emoji Preservation**: Full support for international characters and emoji tags.

### 🔍 Dual-Network Dead Link Verification
- **Dual Verification**: Validates links using both Proxy and Direct connections to eliminate geo-blocking false positives.
- **Categorized Results**: Separates check outputs into `valid.html`, `invalid.html`, `uncertain.html`, and `skipped.html`.
- **Lossless Result Merging**: Merge categorized verification HTML files into a single structured report.

### 🛡️ Safety & Pure Rust Architecture
- **Non-Destructive by Default**: Never modifies original browser databases without explicit confirmation.
- **Zero Shell Dependency**: Replaced shell scripts with pure, multi-threaded Rust binaries.

---

## Quick Start

### 1. Web GUI Mode (Recommended)

Launch the visual dashboard in your browser:

```bash
cargo run -- gui
# or with release binary
bsync gui --port 7890
```

### 2. Command Line Interface (CLI)

```bash
# Build release binary
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync

# List all detected browsers and profile counts
bsync list

# Export all bookmarks from ALL profiles to Downloads (Deduplicated & Merged)
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean --merge

# Run complete pipeline: Export -> Organize -> Check -> Merge
bsync pipeline -o ~/Downloads/pipeline_output

# Check bookmark validity via local proxy
bsync check --file ~/Downloads/all_bookmarks.html --proxy http://127.0.0.1:7890 --limit 200 --export-dir ~/Downloads/check_results

# Merge check results into single HTML
bsync merge-results --dir ~/Downloads/check_results --output ~/Downloads/merged_bookmarks.html
```

---

## Command Reference

| Command | Subcommand / Alias | Description |
|---------|--------------------|-------------|
| `gui` | `ui`, `dashboard` | Start local Web GUI dashboard in browser |
| `list` | `l`, `ls` | List all detected browsers, profiles, and bookmark counts |
| `export` | `e`, `exp` | Export bookmarks across browsers and profiles to HTML/JSON |
| `check` | `c` | Check bookmark URL validity using dual-network validation |
| `merge-results` | `mrg` | Combine check result files (`valid.html` etc.) into single file |
| `pipeline` | `pipe` | Run end-to-end processing pipeline |
| `organize` | `org` | Smart organize bookmarks using AI or built-in rules |
| `analyze` | `ana` | Analyze bookmarks for issues, duplicates, and broken paths |
| `waterfox-cleanup` | `wfc` | Dedicated Waterfox cleanup and export pipeline |
| `backup` | `bak` | Create full database backup of all installed browsers |

---

## Pure Rust Executables

Native binaries located under `src/bin/`:

```bash
# Run one-click full bookmark sync to ~/Downloads/all_bookmarks.html
cargo run --bin sync_now

# Run full processing pipeline
cargo run --bin full_process -- -o ~/Downloads/pipeline_out

# Merge verification results HTML
cargo run --bin merge_results -- -d ~/Downloads/check_results -o ~/Downloads/final_merged.html

# Run dedicated Waterfox cleanup pipeline
cargo run --bin waterfox_cleanup -- -o ~/Downloads/waterfox_clean.html
```

---

## 简体中文说明

> 基于 Rust 构建的高性能 macOS 跨浏览器收藏夹同步、去重、死链检测与全量导出工具 —— 现已支持全新的 Web 可视化控制面板！

### 主要特性

#### 🌐 现代 Web GUI 控制面板 (`bsync gui`)
- **可视化控制台**：运行 `bsync gui` 启动本地暗黑 glassmorphism 风格 Web 控制台。
- **浏览器自动识别**：实时检测已安装浏览器（Safari, Chrome, Brave, Waterfox, LibreWolf, Firefox Nightly）及收藏夹数量。
- **一键导出与合并**：可视化配置导出规则、URL 去重、多配置合并。
- **实时日志流**：内置 SSE（Server-Sent Events）日志终端，实时追踪引擎处理细节。

#### 📚 多浏览器与全 Profile 集中收集
- **支持浏览器**：Safari, Chrome, Brave, Edge, Arc, Waterfox, Firefox, LibreWolf。
- **全 Profile 扫描**：自动扫描 Chromium 与 Gecko 浏览器下所有用户 Profile 目录，将多账号/多配置收藏夹合并归类。
- **Safari 额外支持**：支持导出 Safari 阅读列表（Reading List）为标准书签。

#### ⚡ 智能处理与导出
- **URL 智能去重**：按规范化 URL 自动剔除重复书签。
- **扁平导出**：剥离多余的浏览器根文件夹（如 `Imported > Waterfox > Brave`），保持顶级目录整洁。
- **AI 智能整理**：优先使用本地 Ollama AI 模型（`qwen2.5:7b`）自动分类，无 AI 环境自动退回内置规则引擎。
- ** Unicode/Emoji 保留**：完全保留特殊字符及 Emoji 文件夹名称。

#### 🔍 双网络死链检测
- **双网验证**：同时使用代理与直连双重测试，杜绝因网络屏蔽导致的误判。
- **分类输出**：将结果拆分为 `valid.html`（有效）、`invalid.html`（无效）、`uncertain.html`（不确定）、`skipped.html`（跳过）。
- **无损合并**：提供 `merge-results` 命令将分类文件无损合并为一个标准 HTML 报告。

### CLI 快速使用

```bash
# 启动 GUI 界面
bsync gui

# 命令行全量导出所有 Profile 收藏夹至下载文件夹
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean --merge
```

---

## License

MIT License © 2026 Browser Sync Team
