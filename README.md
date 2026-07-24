# Browser Bookmark Sync (bsync)

> A high-performance, cross-browser bookmark synchronization, deduplication, dead-link checking, and export toolkit for macOS built in Rust. 

[English](#features) | [简体中文](#功能特性)

---

## What is this project?

Managing bookmarks across multiple browsers and profiles is a nightmare. **bsync** solves this by scanning all your installed browsers (Chrome, Firefox, Safari, Brave, Waterfox, LibreWolf, Edge, Arc) across all their user profiles, extracting your bookmarks, history, and cookies, and merging them into a clean, deduplicated, and organized format. 

It also includes advanced features like **AI-powered categorization** (using local Ollama models) and **dual-network dead link detection** to keep your bookmarks fresh and organized. All of this can be controlled via a beautiful **Web GUI Dashboard** or a powerful CLI.

---

## Key Features

### 🌐 Modern Web GUI Dashboard (`bsync gui`)
- **Visual Control Panel**: Launch a local dark-mode glassmorphism Web GUI to manage everything visually.
- **Browser Auto-Detection**: Live scan of installed browsers and exact bookmark counts across all profiles.
- **One-Click Export & Merge**: Visually configure export rules, deduplication, and merge operations.
- **Real-Time Log Stream**: Built-in Server-Sent Events (SSE) log terminal tracking live engine activity.

### 📚 Multi-Browser & All-Profile Support
- **Supported Browsers**: Safari, Chrome, Brave, Edge, Arc, Waterfox, Firefox, LibreWolf.
- **All User Profiles**: Scans all user profiles across Chromium and Gecko browser directories to merge hidden or multi-account bookmarks into one file.
- **Comprehensive Extraction**: Exports standard bookmarks, Safari Reading Lists, Browsing History, and Cookies.

### ⚡ Smart Processing & Exporting
- **URL Deduplication**: Automatically eliminate duplicate links by canonical URL matching.
- **Flat Structure Export**: Strip unnecessary root directory nesting (e.g. `Imported > Waterfox > Brave`) to keep top-level folders clean.
- **Smart Categorization**: Classify bookmarks with local Ollama AI models (`qwen2.5:7b`), with built-in rule engines as fallback.
- **Incremental Updates**: Merge new bookmarks into an existing export file without duplicating.
- **Unicode & Emoji Preservation**: Full support for international characters and emoji tags.

### 🔍 Dual-Network Dead Link Verification
- **Dual Verification**: Validates links using both Proxy and Direct connections to eliminate geo-blocking false positives.
- **Categorized Results**: Separates check outputs into `valid.html`, `invalid.html`, `uncertain.html`, and `skipped.html`.
- **Lossless Result Merging**: Command to merge categorized verification HTML files into a single structured report.

### 🛡️ Safety & Architecture
- **Non-Destructive by Default**: Never modifies original browser databases. Works purely by reading and exporting.
- **100% Rust**: Extremely fast, memory-safe, and multi-threaded processing.

---

## Installation

Requires Rust and Cargo (macOS only, as it uses native browser database formats).

```bash
# Clone the repository
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync

# Build the release binary
cargo build --release

# Optional: Move to your PATH
cp target/release/browser-bookmark-sync /usr/local/bin/bsync
```

---

## Quick Start

### 1. Web GUI Mode (Recommended)

Launch the visual dashboard in your browser:

```bash
# Start the local server (default port 7890)
bsync gui
```

### 2. Command Line Interface (CLI)

```bash
# List all detected browsers and profile counts
bsync list

# Export ALL bookmarks from ALL profiles to Downloads (Deduplicated & Merged)
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean --merge

# Smart organize using local AI
bsync organize --classifier ollama --dry-run
```

---

## Detailed Command Reference

### `bsync export`
Extract data from browsers.

```bash
bsync export [OPTIONS]

Options:
  -o, --output <FILE>      Output path (default: ~/Downloads/bookmarks.html)
  -b, --browsers <LIST>    Source browsers (comma-separated, or 'all')
  -d, --deduplicate        Remove duplicate bookmarks by URL
  -m, --merge              Flatten into a single structure
      --flat               Remove browser root folders (Waterfox, Brave, etc.)
  -r, --reading-list       Include Safari reading list
  -f, --folder <NAME>      Only export a specific folder
      --history            Include browsing history
      --history-days <N>   Days of history (default: 30, 0 = all)
      --cookies            Include cookies
      --clean              Remove empty folders
      --all-profiles       Extract from all browser user profiles
```

### `bsync check`
Verify bookmark URLs using dual-network validation.

```bash
bsync check [OPTIONS]

Options:
  -f, --file <FILE>        HTML bookmark file to check
  -p, --proxy <URL>        Proxy server URL (e.g., http://127.0.0.1:7890)
  -t, --timeout <SECS>     Request timeout (default: 15)
  -c, --concurrency <N>    Concurrent requests (default: 20)
      --export-dir <DIR>   Directory to save categorized results (valid/invalid/uncertain)
      --delete             Delete confirmed invalid bookmarks from the file
```

### `bsync organize`
Merge local files and categorize them smartly.

```bash
# Merge multiple HTML files and organize them
bsync organize -f safari.html brave.html -o merged_organized.html -d --clean

Options:
  -f, --file <FILE>...       Input bookmark files (HTML/JSON). Multiple files are merged.
  -o, --output <OUTPUT>      Output file path
  -d, --deduplicate          Remove duplicate bookmarks by URL (default: true)
      --classifier <MODE>    Classification backend: `auto`, `ollama`, `rules`
      --ollama-model <MODEL> Ollama model name (default: `qwen2.5:7b`)
```

### Pure Rust Pipeline Executables
Native binaries located under `src/bin/` for specific full-pipeline workflows:

```bash
# Run one-click full bookmark sync to ~/Downloads/all_bookmarks.html
cargo run --bin sync_now

# Run full processing pipeline (Export -> Organize -> Check -> Merge)
cargo run --bin full_process -- -o ~/Downloads/pipeline_out

# Merge verification results HTML
cargo run --bin merge_results -- -d ~/Downloads/check_results -o ~/Downloads/final_merged.html
```

---
---

## 简体中文说明

> 基于 Rust 构建的高性能 macOS 跨浏览器收藏夹同步、去重、死链检测与全量导出工具。

## 项目简介

在多个浏览器和用户配置文件（Profile）之间管理收藏夹通常是一场噩梦。**bsync** 旨在彻底解决这个问题。它会自动扫描您系统中安装的所有浏览器（Chrome、Firefox、Safari、Brave、Waterfox、LibreWolf、Edge、Arc）以及它们的**所有用户配置文件**，提取书签、历史记录和 Cookie，并将它们合并为一个干净、去重且结构合理的 HTML 文件。

此外，它还包含高级功能，例如**AI 智能分类**（使用本地 Ollama 模型）和**双网络死链检测**，以保持您的书签库始终新鲜有序。所有这些功能都可以通过美观的 **Web GUI 控制台** 或强大的命令行界面（CLI）来控制。

---

## 核心功能

### 🌐 现代 Web GUI 控制面板 (`bsync gui`)
- **可视化控制台**：运行 `bsync gui` 启动本地暗黑 glassmorphism 风格 Web 控制台。
- **浏览器自动识别**：实时检测已安装浏览器及所有 Profile 的收藏夹数量。
- **一键导出与合并**：可视化配置导出规则、URL 去重、多配置合并。
- **实时日志流**：内置 SSE（Server-Sent Events）日志终端，实时追踪引擎处理细节。

### 📚 多浏览器与全 Profile 集中收集
- **支持浏览器**：Safari, Chrome, Brave, Edge, Arc, Waterfox, Firefox, LibreWolf。
- **全 Profile 扫描**：自动扫描 Chromium 与 Gecko 浏览器下所有用户 Profile 目录，将多账号/多配置收藏夹无缝合并。
- **全面数据提取**：支持导出标准书签、Safari 阅读列表（Reading List）、浏览历史记录和 Cookies。

### ⚡ 智能处理与导出
- **URL 智能去重**：按规范化 URL 自动剔除重复书签。
- **扁平导出 (`--flat`)**：剥离多余的浏览器根文件夹（如 `Imported > Waterfox > Brave`），让您的自定义文件夹直接显示在顶层。
- **AI 智能整理**：优先使用本地 Ollama AI 模型（`qwen2.5:7b`）自动分类，无 AI 环境自动退回内置 JSON 规则引擎。
- **增量更新**：将新书签合并到现有的导出文件中，且不产生重复。
- **Unicode/Emoji 保留**：完全保留特殊字符及 Emoji 文件夹名称。

### 🔍 双网络死链检测 (`bsync check`)
- **双网验证**：同时使用代理与直连双重测试，杜绝因地域网络屏蔽导致的“假死链”误判。
- **分类输出**：将检查结果智能拆分为 `valid.html`（有效）、`invalid.html`（无效）、`uncertain.html`（不确定）、`skipped.html`（跳过）。
- **无损合并**：提供专用命令将上述分类文件无损合并为一个保留原有目录结构的标准 HTML 报告。

### 🛡️ 安全性与纯 Rust 架构
- **默认非破坏性**：仅执行读取和导出操作，绝不修改您浏览器的原始数据库文件。
- **100% Rust**：极致的运行速度、内存安全以及多线程并发处理能力。

---

## 快速开始

### 1. Web GUI 模式 (推荐)

在浏览器中启动可视化面板：

```bash
# 启动本地服务 (默认端口 7890)
bsync gui
```

### 2. 命令行模式 (CLI)

```bash
# 列出检测到的浏览器及书签数量
bsync list

# 全量导出所有浏览器的所有 Profile 收藏夹至下载文件夹（自动去重、合并、扁平化）
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean --merge

# 使用本地 AI 进行智能分类预览
bsync organize --classifier ollama --dry-run
```

---

## 详细命令说明

### `bsync export` (导出书签)
从浏览器提取数据。

```bash
bsync export [OPTIONS]

选项:
  -o, --output <FILE>      输出路径 (默认: ~/Downloads/bookmarks.html)
  -b, --browsers <LIST>    指定浏览器 (逗号分隔，或 'all')
  -d, --deduplicate        按 URL 移除重复书签
  -m, --merge              将所有文件夹扁平合并
      --flat               移除浏览器顶层名称文件夹
  -r, --reading-list       包含 Safari 阅读列表
  -f, --folder <NAME>      仅导出特定名称的文件夹
      --history            包含浏览历史
      --cookies            包含 Cookies
      --clean              清理空文件夹
      --all-profiles       扫描并提取所有用户 Profile 目录
```

### `bsync check` (死链检测)
使用双网络验证检查书签 URL 的有效性。

```bash
bsync check [OPTIONS]

选项:
  -f, --file <FILE>        要检查的 HTML 书签文件
  -p, --proxy <URL>        代理服务器地址 (例如: http://127.0.0.1:7890)
  -t, --timeout <SECS>     请求超时时间 (默认: 15)
  -c, --concurrency <N>    并发请求数 (默认: 20)
      --export-dir <DIR>   分类结果 (有效/无效/不确定) 的导出目录
      --delete             直接在文件中删除确认无效的书签
```

### `bsync organize` (智能整理)
合并本地 HTML 文件并进行智能分类。

```bash
# 合并多个 HTML 文件并整理
bsync organize -f safari.html brave.html -o merged_organized.html -d --clean

选项:
  -f, --file <FILE>...       输入书签文件 (HTML/JSON)。支持指定多个文件进行合并。
  -o, --output <OUTPUT>      输出文件路径
  -d, --deduplicate          按 URL 移除重复书签 (默认: true)
      --classifier <MODE>    分类引擎: `auto` (自动), `ollama` (AI), `rules` (内置规则)
      --ollama-model <MODEL> Ollama 模型名称 (默认: `qwen2.5:7b`)
```

### 原生 Rust 管道工具 (`src/bin/`)
用于执行特定全流程的二进制文件：

```bash
# 一键同步并导出全量书签到 ~/Downloads/all_bookmarks.html
cargo run --bin sync_now

# 运行完整处理管道 (导出 -> 整理 -> 检查 -> 合并)
cargo run --bin full_process -- -o ~/Downloads/pipeline_out

# 将检查结果合并为单一 HTML
cargo run --bin merge_results -- -d ~/Downloads/check_results -o ~/Downloads/final_merged.html
```

---

## License

MIT License © 2026 Browser Sync Team
