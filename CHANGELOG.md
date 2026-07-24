# Changelog

## [2026-07-24] v1.3.0

### New Features
- **Web GUI Control Center (`bsync gui` / `bsync ui`)**: Added a built-in dark-mode Web Dashboard accessible via browser, supporting one-click browser detection, visual bookmark export, URL checking, and result merging with live SSE log streaming.
- **Multi-Profile Export**: Supported extracting bookmarks across all user profiles for Chrome, Firefox, Waterfox, LibreWolf, Brave, and Safari.
- **Pure Rust Pipelines**: Replaced legacy shell scripts with pure Rust binaries and subcommands (`pipeline`, `merge-results`, `waterfox-cleanup`, `sync_now`).

### Improvements & Fixes
- **Strict HTML Structure Parity**: Fixed wrapper stripping in `merge_check_result_files` so result merging precisely unwraps `📁镜像文件夹` root tags.
- **Subdirectory Reorganization**: Cleaned up workspace into `src/assets`, `src/bin`, `src/gui`, `src/tests`, `src/examples`.
- **Safari Reading List & LibreWolf Support**: Improved Firefox-family SQLite parsing for LibreWolf and Waterfox.

---

## 更新日志

## [2026-07-24] v1.3.0

### 新功能
- **Web GUI 可视化控制面板 (`bsync gui` / `bsync ui`)**: 内置暗黑 macOS 风格 Web Dashboard，支持浏览器一键检测、可视导出、结果合并以及 SSE 实时日志流。
- **全 Profile 收集**: 支持从 Chrome、Firefox、Waterfox、LibreWolf、Brave、Safari 的所有用户 Profile 中集中导出收藏夹。
- **纯 Rust 子命令体系**: 全面替代 Shell 脚本，提供原生 Rust `pipeline`、`merge-results`、`waterfox-cleanup` 和 `sync_now` 工具链。

### 优化与修复
- **HTML 拆解等价性修复**: 修复 `merge_check_result_files` 在合并结果文件时未剥离 `📁镜像文件夹` 顶层 wrapper 的问题，确保与旧脚本 100% 结构等价。
- **项目目录架构整理**: 重新归类并整理为 `src/assets`、`src/bin`、`src/gui`、`src/tests`、`src/examples` 子目录规范。
- **LibreWolf 与 Waterfox 全面兼容**: 完善基于 SQLite/JSON 的 Firefox 系多配置项自动扫描与数据抽取。

---

## [2024-12-20] v1.2.0

### New Features
- **Quick Temp Processing** (`quick_temp.sh`): One-click script to extract temp folders from all browsers, organize, check dead links, and merge output
- **Merge Results** (`merge_results.sh`): Combine check results into single HTML with categorized folders

### Improvements
- **Folder Structure Preservation**: HTML import now correctly parses `<DT><H3>` folder tags using stack-based parsing
- **Dead Link Detection**: 403/503/429 responses now treated as valid (server online, browser accessible)
- **Reduced Uncertain Rate**: From 87% to 3.7% by optimizing validation logic
- **Accurate Bookmark Count**: Fixed log display using recursive count instead of array length

### Bug Fixes
- Fixed URL checker hanging at 23516/23520 by adding `connect_timeout` and `pool_idle_timeout`
- Fixed manifest path error in `smart_build.sh` for modern_format_boost
