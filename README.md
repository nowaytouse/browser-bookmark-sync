# Browser Bookmark Sync (bsync)

> A rigorous, pure-Rust toolkit for macOS designed to extract, deduplicate, validate, and organize bookmarks across all major browsers and user profiles.

[English](#features) | [简体中文](#功能特性)

---

## Overview

**bsync** is a specialized macOS tool that solves the fragmentation of bookmark management. Instead of manually exporting HTML files from each browser, `bsync` directly reads the underlying databases (SQLite for Gecko, JSON for Chromium, Plist for WebKit) across **all user profiles**.

It aggregates your bookmarks, mathematically deduplicates them by URL, validates them against link rot using a dual-network approach (direct + proxy), and intelligently categorizes them using local AI (Ollama) or regex rules. It outputs a clean, unified Netscape HTML file ready for import.

## Core Capabilities

### 🌐 Native Web GUI Dashboard

- Start a local `axum`-powered web server with `bsync gui`.
- Provides a dark-mode, glassmorphism interface for visual control over exports and validations.
- Features real-time log streaming directly to the browser via Server-Sent Events (SSE).

### 🛠 High-Fidelity & Auditable Pipeline Commands

- `bsync inventory`: Enumerate detected browser installations, user profiles, and bookmark counts across macOS.
- `bsync collect --output-dir ./run`: Harvest original un-deduplicated bookmark tree and export `bookmarks-original.html` preserving full browser/profile/folder hierarchy.
- `bsync dedupe --output-dir ./run`: Perform deterministic exact URL deduplication, exporting `bookmarks-deduplicated.html` and sidecar audit reports (`inventory.json`, `duplicates.json`, `duplicates.csv`, `run-manifest.json`).
- `bsync pipeline`: Execute complete end-to-end collection, snapshot isolation, exact deduplication, and sidecar generation in one step.

### 📚 Deep Database Extraction (All Profiles)

- **Chromium** (Chrome, Brave, Edge, Arc): Parses `Bookmarks` JSON files.
- **Gecko** (Firefox, Waterfox, LibreWolf): Safely queries `places.sqlite` using WAL mode to extract bookmark trees.
- **WebKit** (Safari): Parses `Bookmarks.plist`, including native support for extracting the Safari Reading List as standard bookmarks.
- **Multi-Profile Support**: Discovers named/root-level Chromium profiles, Firefox `profiles.ini` entries (including external absolute paths), and bounded unknown Chromium/Gecko forks. Collection aborts instead of silently publishing a partial harvest when any discovered profile cannot be snapshotted or parsed.

### 🔍 Same-Day Bookmark Health Validation

`bsync check` uses the conservative health engine by default. It distinguishes a dead website from a live site whose saved post/video/resource disappeared, and stores every real probe with its timestamp in `./bookmark-health.sqlite` (override with `--history-db`).

- Only evidence from the latest rolling 24-hour window participates in a verdict.
- Normal site/content death requires three completed runs in that window and two independent verifier families. Three dual-resolver `NXDOMAIN` runs or three high-confidence `ContentMissing` runs are the narrow single-family exceptions.
- Any real, uncached, content-aware success immediately keeps the bookmark as `Live`/`Recovered`, even if another probe disagrees.
- Doubtful URLs are retried per URL up to three times (5 seconds by default; interval hard-capped at 60 seconds) before manual review. Oversized/truncated bodies remain review-only.
- Proxy mode disables direct DNS probes and only permits proxy-aware secondary verification.
- `--delete` only removes `ConfirmedDead` URLs from an HTML input into a separate atomic output copy; it never rewrites browser profiles directly.

### 🤖 Smart Categorization (AI & Rules)

- **Local AI**: Connects to a local Ollama instance (default: `qwen2.5:7b`) to semantically categorize bookmarks based on their titles and URLs.
- **Regex Rule Engine**: If AI is unavailable, falls back to a highly configurable JSON-based regex engine that matches domains, paths, and titles.

### 🧹 Algorithmic Cleanup

- **Deduplication**: Flattens the bookmark tree, calculates unique URL hashes, and removes duplicates while prioritizing the preservation of user-created folder structures.
- **Flat Export**: Strips out redundant browser-generated root folders (e.g., "Imported from Safari") so your custom folders sit cleanly at the top level.
- **Empty Folder Pruning**: Recursively removes orphaned or empty directories after deduplication.

---

## Installation & Build

`bsync` is built entirely in Rust. It requires macOS as it relies on specific macOS application support paths.

```bash
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync

# Compile release binary
cargo build --release

# Make globally available
cp target/release/browser-bookmark-sync /usr/local/bin/bsync
```

---

## Quick Start

### GUI Mode

```bash
bsync gui --port 7890
```

### CLI Workflows

**1. Export and Deduplicate Everything**
Extract bookmarks from all installed browsers and profiles, remove duplicates, strip root folders, and output to a single HTML file:

```bash
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean
```

**2. Validate URLs (Dead Link Check)**
Check an exported HTML file for dead links, utilizing a local proxy for blocked sites:

```bash
bsync check --file ~/Downloads/all_bookmarks.html --export-dir ~/Downloads/check_results
# The same command finishes its bounded same-day retries; delete only ConfirmedDead items:
bsync check --file ~/Downloads/all_bookmarks.html --delete --output ~/Downloads/bookmarks-cleaned.html
```

**3. AI Categorization**
Organize bookmarks using a local Ollama model:

```bash
bsync organize -f ~/Downloads/all_bookmarks.html -o ~/Downloads/organized.html --classifier ollama
```

---

## CLI Reference

Run `bsync --help` or `bsync <COMMAND> --help` for full details.

- `bsync export`: Extract, deduplicate, and format bookmarks from browser databases.
- `bsync check`: Dual-network URL validation.
- `bsync organize`: Merge local HTML files and categorize via AI/Rules.
- `bsync analyze`: Dry-run analysis of bookmark tree health (duplicates, empty folders).
- `bsync validate`: Strict parsing validation of an HTML bookmark file.
- `bsync gui`: Launch the local Web UI.
- `bsync list`: Enumerate detected browsers, profiles, and their respective bookmark counts.
- `bsync pipeline`: Run a full automated flow (Export -> Organize -> Check -> Merge).
- `bsync merge-results`: Combine `check` results (`valid.html`, `invalid.html`) back into a unified file.

_(Note: Highly experimental flags such as `history`, `cookies`, and `export-data` exist in the CLI but are strictly for visualization or developer debugging and are not meant for daily data synchronization.)_

---

---

## 简体中文说明

> 基于纯 Rust 构建的 macOS 极客工具，专为跨浏览器、跨 Profile 的书签提取、去重、死链验证与 AI 整理而设计。

## 核心架构与原理

**bsync** 并非简单的书签格式转换器。它通过直接读取底层数据库（Gecko 的 SQLite、Chromium 的 JSON、WebKit 的 Plist），彻底解决了跨浏览器书签管理的碎片化问题。它支持自动遍历所有的用户配置文件（Profile），将分散的数据聚合，进行数学级别的 URL 去重，并通过双网络环境检测死链，最后使用本地大模型（Ollama）进行智能分类。

### 🌐 原生 Web GUI

- 运行 `bsync gui` 启动基于 `axum` 的本地 Web 服务器。
- 提供暗黑模式的玻璃态 UI 界面，实现可视化的导出和验证操作。
- 通过 Server-Sent Events (SSE) 技术，将引擎底层的处理日志实时流式传输至浏览器前端。

### 📚 底层数据库级提取 (全 Profile 支持)

- **Chromium** (Chrome, Brave, Edge, Arc): 原生解析 `Bookmarks` JSON 树。
- **Gecko** (Firefox, Waterfox, LibreWolf): 使用 WAL 模式安全挂载 `places.sqlite` 并提取书签数据。
- **WebKit** (Safari): 解析 `Bookmarks.plist`，原生支持将 Safari 阅读列表（Reading List）转化为标准书签导出。
- **多配置扫描**: 自动发现命名/根级 Chromium Profile、Firefox `profiles.ini`（含外部绝对路径）及受限深度内的未知 Chromium/Gecko 分支；任一已发现 Profile 无法稳定快照或解析时会整体报错，不会静默产出残缺结果。

### 🔍 当天内完成的失效收藏夹验证

`bsync check` 默认启用保守健康引擎。它会区分“网站整体死亡”和“网站仍在但收藏的帖子、视频或深层内容消失”，并把每次真实探测及时间写入 `./bookmark-health.sqlite`（可用 `--history-db` 改路径）。

- 只有最近滚动 24 小时内的证据会参与判定，不再等待多天历史。
- 常规的网站/内容死亡需要在窗口内完成 3 轮探测，并由 2 个独立验证器家族形成证据；3 轮双解析器 `NXDOMAIN`，或 3 轮高置信 `ContentMissing`，是严格限定的单家族例外。
- 任一真实、非缓存且读取到有效正文的成功访问都会立即作为 `Live`/`Recovered` 保留，即使另一探针意见冲突。
- 存疑 URL 会按单个网址最多快速复探 3 轮（默认间隔 5 秒，硬上限 60 秒）再进入人工复核；超大或截断正文只进入复核。
- 代理模式会禁用直连 DNS 探针，并只允许明确支持代理的二次验证器。
- `--delete` 只把历史确认死亡的 URL 从 HTML 输入精确移除并原子写入独立副本，不直接改写浏览器 Profile。

### 🤖 智能分类引擎 (AI大模型 + 正则)

- **本地 AI**: 接入本地运行的 Ollama 实例（默认调用 `qwen2.5:7b`），通过语义理解书签标题与 URL 进行自动化归类。
- **正则规则引擎**: 在无 AI 环境下，自动降级至基于 JSON 配置的高性能正则表达式引擎，按域名、路径和标题进行精准匹配。

### 🧹 算法级清理

- **严格去重**: 在拍平书签树后计算 URL 唯一哈希，剔除重复项，并在保留原有用户目录结构的权重上进行智能取舍。
- **扁平化输出**: 自动剥离浏览器自动生成的恶心根目录（如“从 Safari 导入”），确保您的自定义文件夹位于 HTML 最顶层。
- **孤儿目录修剪**: 在完成去重和分类后，递归扫描并销毁所有空文件夹。

---

## 快速使用指引

### GUI 可视化模式

```bash
bsync gui --port 7890
```

### 核心 CLI 工作流

**1. 全量提取与极致清理**
遍历所有浏览器及分身，提取书签，去重，扁平化目录并清理空文件夹，最后输出为一个标准 HTML：

```bash
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles --flat -d --clean
```

**2. 死链检测 (借助代理排雷)**
对导出的 HTML 文件进行双网络死链检测，隔离出真正的无效链接：

```bash
bsync check --file ~/Downloads/all_bookmarks.html --export-dir ~/Downloads/check_results
# 同一次命令完成当天内的有限复探；仅删除 ConfirmedDead：
bsync check --file ~/Downloads/all_bookmarks.html --delete --output ~/Downloads/bookmarks-cleaned.html
```

**3. AI 智能整理**
调用本地大模型重新梳理凌乱的书签堆：

```bash
bsync organize -f ~/Downloads/all_bookmarks.html -o ~/Downloads/organized.html --classifier ollama
```

_(注：CLI 中包含的 `history`, `cookies`, `export-data` 等命令为实验性开发者功能，用于数据可视化与调试，并非核心书签同步功能。)_

---

## License

MIT License © 2026 Browser Sync Team
