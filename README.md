# Browser Bookmark Sync (bsync)

> A high-performance, cross-browser bookmark synchronization, deduplication, dead-link checking, and export toolkit for macOS built in Rust. 

[English](#features) | [简体中文](#功能特性)

---

## What is this project?

Managing bookmarks, history, passwords, and cookies across multiple browsers and profiles is a nightmare. **bsync** is a pure Rust CLI and GUI toolkit built specifically for macOS that solves this. It automatically scans all your installed browsers (Chrome, Firefox, Safari, Brave, Waterfox, LibreWolf, Edge, Arc) across **all their user profiles**, extracting and merging your data into clean, deduplicated, and organized formats. 

It includes advanced features like **AI-powered categorization** (using local Ollama models), **dual-network dead link detection**, and **sensitive data extraction** (passwords, cookies). All of this can be controlled via a beautiful **Web GUI Dashboard** or a highly extensive CLI.

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
- **Comprehensive Extraction**: Exports standard bookmarks, Safari Reading Lists, Browsing History, Passwords (with macOS keychain access), and Cookies.

### ⚡ Smart Processing & Exporting
- **URL Deduplication**: Automatically eliminate duplicate links by canonical URL matching.
- **Flat Structure Export**: Strip unnecessary root directory nesting (e.g. `Imported > Waterfox > Brave`) to keep top-level folders clean.
- **Smart Categorization**: Classify bookmarks with local Ollama AI models (`qwen2.5:7b`), with built-in rule engines as fallback.
- **Unicode & Emoji Preservation**: Full support for international characters and emoji tags.

### 🔍 Dual-Network Dead Link Verification
- **Dual Verification**: Validates links using both Proxy and Direct connections to eliminate geo-blocking false positives.
- **Categorized Results**: Separates check outputs into `valid.html`, `invalid.html`, `uncertain.html`, and `skipped.html`.

### 🛡️ Safety & Architecture
- **Non-Destructive by Default**: Never modifies original browser databases. Works purely by reading and exporting.
- **100% Rust**: Extremely fast, memory-safe, and multi-threaded processing.

---

## Quick Start

### 1. Web GUI Mode (Recommended)

Launch the visual dashboard in your browser:

```bash
cargo run -- gui
# Or if installed globally:
bsync gui --port 7890
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

## Full CLI Command Reference

`bsync` features an extensive suite of commands. Below is the complete reference.

### 1. Extraction & Synchronization
- **`bsync export [OPTIONS]`**: Export browser data to HTML/JSON (Bookmarks, History, Cookies).
  - `--all-profiles` / `-a`: Extract from all user profiles.
  - `--browsers` / `-b`: Target specific browsers.
  - `--history`, `--history-days`: Include browsing history.
  - `--cookies`, `--passwords`: Include sensitive data.
  - `--flat`: Remove browser root folders.
  - `--deduplicate` / `-d`: Remove duplicate URLs.
  - `--merge` / `-m`: Flatten into a single structure.
  - `--reading-list` / `-r`: Include Safari Reading List.
  - `--folder` / `-f`: Extract only a specific folder.
- **`bsync export-data [OPTIONS]`**: Specifically extract sensitive data.
  - Options: `--passwords`, `--cookies`, `--downloads`, `--format csv/json/netscape`.
- **`bsync history [OPTIONS]`**: Sync browsing history between browsers.

### 2. Validation & Checking
- **`bsync check [OPTIONS]`**: Dual-network URL validity checking.
  - `--file`: Input HTML file to check.
  - `--proxy`: Set local proxy (e.g., `http://127.0.0.1:7890`).
  - `--timeout`, `--concurrency`: Tune network settings.
  - `--export-dir`: Save categorized valid/invalid/uncertain results here.
- **`bsync analyze`**: Scan bookmarks for duplicates, empty folders, and structural issues.
- **`bsync validate`**: Verify core bookmark integrity and syntax.

### 3. Organization & Utilities
- **`bsync organize [OPTIONS]`**: Merge local files and categorize them via AI or rules.
  - `--classifier ollama / auto / rules`: Pick categorization engine.
  - `--ollama-model`: (Default: `qwen2.5:7b`).
- **`bsync rules`**: List all built-in classification rules.
- **`bsync backup`**: Create a massive full-database backup of all installed browsers.

### 4. Automated Pipelines
- **`bsync pipeline`**: Runs Export -> Organize -> Check -> Merge in one automated flow.
- **`bsync merge-results`**: Merges categorized check results back into a single valid HTML file.
- **`bsync waterfox-cleanup`**: Dedicated Waterfox-specific cleanup pipeline.

---

## Native Pipeline Executables
For convenience, `src/bin/` contains pure-Rust native entry points for common workflows:

```bash
# One-click sync to ~/Downloads/all_bookmarks.html
cargo run --bin sync_now

# Full pipeline execution
cargo run --bin full_process -- -o ~/Downloads/pipeline_out

# Merge check results
cargo run --bin merge_results -- -d ~/Downloads/check_results -o ~/Downloads/final_merged.html
```

---
---

## 简体中文说明

> 基于 Rust 构建的高性能 macOS 跨浏览器收藏夹同步、去重、死链检测、数据提取与全量导出工具。

## 项目简介

在多个浏览器和用户配置文件（Profile）之间管理数据通常是一场噩梦。**bsync** 是一个专为 macOS 打造的纯 Rust CLI 和 GUI 工具包，旨在彻底解决这个问题。它会自动扫描您系统中安装的所有浏览器（Chrome、Firefox、Safari、Brave、Waterfox、LibreWolf、Edge、Arc）以及它们的**所有用户配置文件**，提取书签、历史记录、Cookies 和密码，并将它们合并为干净、去重且结构合理的格式。

它包含了诸如**AI 智能分类**（使用本地 Ollama 模型）、**双网络死链检测**以及**敏感数据提取**等高级功能。所有这些功能都可以通过美观的 **Web GUI 控制台** 或非常强大的命令行界面（CLI）来控制。

---

## 核心功能

### 🌐 现代 Web GUI 控制面板 (`bsync gui`)
- **可视化控制台**：启动本地暗黑 glassmorphism 风格 Web 控制台，可视化管理。
- **浏览器自动识别**：实时检测已安装浏览器及所有 Profile 的收藏夹数量。
- **一键导出与合并**：可视化配置导出规则、URL 去重、多配置合并。
- **实时日志流**：内置 SSE（Server-Sent Events）日志终端，实时追踪引擎处理细节。

### 📚 多浏览器与全 Profile 集中收集
- **支持浏览器**：Safari, Chrome, Brave, Edge, Arc, Waterfox, Firefox, LibreWolf。
- **全 Profile 扫描**：自动扫描 Chromium 与 Gecko 浏览器下所有用户 Profile 目录，将多账号/多配置收藏夹无缝合并。
- **全面数据提取**：支持导出标准书签、Safari 阅读列表、浏览历史记录、密码（接入 macOS 钥匙串）和 Cookies。

### ⚡ 智能处理与导出
- **URL 智能去重**：按规范化 URL 自动剔除重复书签。
- **扁平导出 (`--flat`)**：剥离多余的浏览器根文件夹（如 `Imported > Waterfox > Brave`），让您的自定义文件夹直接显示在顶层。
- **AI 智能整理**：优先使用本地 Ollama AI 模型（`qwen2.5:7b`）自动分类，无 AI 环境自动退回内置规则引擎。
- **Unicode/Emoji 保留**：完全保留特殊字符及 Emoji 文件夹名称。

### 🔍 双网络死链检测 (`bsync check`)
- **双网验证**：同时使用代理与直连双重测试，杜绝因地域网络屏蔽导致的“假死链”误判。
- **分类输出**：将检查结果智能拆分为 `valid.html`（有效）、`invalid.html`（无效）、`uncertain.html`（不确定）、`skipped.html`（跳过）。

### 🛡️ 安全性与架构
- **默认非破坏性**：仅执行读取和导出操作，绝不修改原始数据库。
- **100% Rust**：极致的运行速度、内存安全以及多线程并发处理能力。

---

## 快速开始

### 1. Web GUI 模式 (推荐)

```bash
# 启动本地服务 (默认端口 7890)
cargo run -- gui
```

### 2. 命令行模式 (CLI)

```bash
# 列出检测到的浏览器及书签数量
bsync list

# 全量导出所有浏览器的所有 Profile 收藏夹至下载文件夹（自动去重、合并、扁平化）
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean --merge
```

---

## 完整 CLI 命令参考

`bsync` 拥有极其丰富的子命令体系。以下为完整功能参考：

### 1. 提取与同步
- **`bsync export [OPTIONS]`**：全能导出工具（支持书签、历史、Cookies）。
  - `--all-profiles` / `-a`: 扫描提取所有用户的 Profile。
  - `--history`: 提取浏览历史。
  - `--cookies`, `--passwords`: 提取敏感数据。
  - `--flat`: 移除顶层浏览器名称文件夹，实现扁平化。
  - `--deduplicate` / `-d`: 按 URL 去重。
  - `--merge` / `-m`: 文件夹跨浏览器合并。
  - `--reading-list` / `-r`: 提取 Safari 阅读列表。
- **`bsync export-data [OPTIONS]`**：专门用于敏感数据的格式化导出（支持导出为 CSV / JSON / Netscape 格式的密码、Cookies、下载历史）。
- **`bsync history [OPTIONS]`**：在不同浏览器间同步浏览历史。

### 2. 验证与检查
- **`bsync check [OPTIONS]`**：双网络 URL 有效性双重测试。
  - `--file`: 指定要检查的 HTML 导出文件。
  - `--proxy`: 指定本地代理（如 `http://127.0.0.1:7890`）。
  - `--export-dir`: 将有效、无效、不确定结果分类导出到该目录。
- **`bsync analyze`**：深度扫描并分析收藏夹结构缺陷、空文件夹及重复项。
- **`bsync validate`**：验证底层书签完整性。

### 3. 整理与实用工具
- **`bsync organize [OPTIONS]`**：合并本地多个文件并借助 AI 重新分类。
  - `--classifier ollama / rules`: 指定分类器后端。
- **`bsync rules`**：打印并列出所有内置的 JSON 分类规则。
- **`bsync backup`**：一键为系统中所有浏览器数据库建立完整备份。

### 4. 全自动流水线
- **`bsync pipeline`**：一键执行 导出 -> 整理 -> 检查 -> 结果合并 的全自动工作流。
- **`bsync merge-results`**：将 `check` 命令生成的分类结果无损合并回单个标准 HTML。
- **`bsync waterfox-cleanup`**：针对 Waterfox 浏览器的专用清理脚本。

---

## 原生 Rust 流水线执行程序
为了方便调用，`src/bin/` 下提供了纯 Rust 编写的直接执行入口：

```bash
# 一键同步并导出全量书签到 ~/Downloads/all_bookmarks.html
cargo run --bin sync_now

# 运行完整自动化流水线
cargo run --bin full_process -- -o ~/Downloads/pipeline_out

# 无损合并检查结果
cargo run --bin merge_results -- -d ~/Downloads/check_results -o ~/Downloads/final_merged.html
```

---

## License
MIT License © 2026 Browser Sync Team
