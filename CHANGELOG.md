# Changelog

All notable changes to this project will be documented in this file.

## [1.3.0] - 2026-07-24

### Added
- **Web GUI Dashboard (`bsync gui` / `bsync ui`)**: Native dark-mode glassmorphism Web Dashboard powered by `axum` and embedded HTML/CSS/JS frontend. Includes one-click browser detection, visual bookmark export, URL validation controls, and result merging.
- **Real-Time Log Stream**: Server-Sent Events (SSE) log terminal built into the GUI to stream engine activity in real time.
- **Multi-Profile Bookmark Collection**: Support scanning and extracting bookmarks across all user profiles for Chrome, Firefox, Waterfox, LibreWolf, Brave, and Safari.
- **Pure Rust Binary Tools**: Added subcommands (`pipeline`, `merge-results`, `waterfox-cleanup`, `sync_now`) replacing legacy shell scripts.

### Changed
- **Folder Architecture Cleanup**: Standardized repository layout into `src/assets`, `src/bin`, `src/gui`, `src/tests`, and `src/examples`.
- **Default Executable Target**: Added `default-run = "browser-bookmark-sync"` to `Cargo.toml` for seamless `cargo run` execution.

### Fixed
- **HTML Structure Parity**: Fixed wrapper stripping in `merge_check_result_files` to properly unwrap `📁镜像文件夹` root tags, ensuring 100% output equivalence.
- **Firefox Derivative Scanning**: Improved SQLite database parsing for LibreWolf and Waterfox profiles.

---

## [1.2.0] - 2024-12-20

### Added
- **Quick Processing Pipeline**: Automated batch pipeline to extract, organize, check dead links, and merge bookmarks across all browsers.
- **Result Merging Utility**: Command to merge categorized check results (`valid.html`, `invalid.html`, etc.) into a unified report.

### Improved
- **Folder Structure Preservation**: HTML parser now correctly handles `<DT><H3>` tags using stack-based node parsing.
- **Dead Link Verification**: HTTP 403, 503, and 429 status codes are now recognized as valid endpoints.
- **Reduced False Positives**: Reduced verification uncertainty rate from 87% to 3.7%.
- **Accurate Bookmark Counting**: Switched to recursive tree counting for precise log metrics.

### Fixed
- Fixed URL checker connection freeze by setting strict connection and idle pool timeouts.
