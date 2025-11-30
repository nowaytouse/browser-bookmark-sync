# 🔄 Browser Bookmark Sync

A reliable cross-browser synchronization tool for bookmarks, history, and cookies. Uses a **Hub Browser Architecture** to prevent data duplication and maintain organization.

[中文文档](./README_CN.md)

## ✨ Features

- 🎯 **Hub Browser Architecture** - Designate primary browsers, sync between them, optionally clean others
- 📚 **Full Data Sync** - Bookmarks, history, reading lists, and cookies in one command
- 🌳 **Preserves Structure** - Complete folder hierarchy maintained, no flattening
- 🔄 **Deduplication** - Automatic removal of duplicate URLs and entries
- 🔒 **Safe Backups** - Automatic backup before every operation
- 🧪 **Tested & Verified** - Integration test suite included

## 🖥️ Supported Browsers

| Browser | Bookmarks | History | Cookies |
|---------|-----------|---------|---------|
| **Brave Nightly** | ✅ | ✅ | ✅ |
| **Waterfox** | ✅ | ✅ | ✅ |
| **Brave** | ✅ | ✅ | ✅ |
| **Chrome** | ✅ | ✅ | ✅ |
| **Safari** | ✅ | ✅ | - |
| **Firefox** | ✅ | ✅ | ✅ |
| **LibreWolf** | ✅ | ✅ | ✅ |

## 🚀 Quick Start

### One-Click Sync (macOS)

Double-click `sync-now.command`:

```bash
./sync-now.command
```

### Command Line

```bash
# Full sync between hub browsers (bookmarks + history + cookies)
browser-bookmark-sync sync

# Preview changes without executing
browser-bookmark-sync sync --dry-run

# Sync and clear non-hub browsers
browser-bookmark-sync sync --clear-others

# Custom hub browsers
browser-bookmark-sync sync --browsers "chrome,firefox"

# List detected browsers
browser-bookmark-sync list

# Validate data integrity
browser-bookmark-sync validate
```

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
│        Chrome | Brave | Safari | LibreWolf          │
│              (Data migrated then cleared)           │
└─────────────────────────────────────────────────────┘
```

### What Gets Synced

| Data Type | Sync Method |
|-----------|-------------|
| **Bookmarks** | Uses browser with best folder structure as base, preserves hierarchy |
| **History** | Merges ALL history from all browsers, deduplicates by URL |
| **Cookies** | Merges cookies, deduplicates by host+name+path |
| **Reading Lists** | Safari reading list → Hub browser bookmark folder |

## 📊 Commands Reference

| Command | Description |
|---------|-------------|
| `sync` | **Full sync** - bookmarks + history + cookies between hub browsers |
| `sync --clear-others` | Full sync + clear non-hub browser data |
| `sync-history` | Sync ALL history only |
| `sync-cookies` | Sync cookies only |
| `validate` | Check data integrity across all browsers |
| `list` | Show detected browsers and paths |
| `schedule` | Start automatic periodic syncing |

### Sync Options

```bash
browser-bookmark-sync sync [OPTIONS]

Options:
  -b, --browsers <BROWSERS>  Hub browsers [default: waterfox,brave-nightly]
      --clear-others         Clear data from non-hub browsers
      --all-profiles         Read from all browser profiles (slower, may have duplicates)
  -d, --dry-run              Preview without making changes
  -v, --verbose              Detailed output
```

### Performance

By default, only the **Default profile** is read from each browser for optimal performance. Use `--all-profiles` to read from all profiles when needed:

```bash
# Fast mode (default) - reads only Default profile
browser-bookmark-sync sync --dry-run
# ~1.1s for 41,661 bookmarks

# All profiles mode - reads all browser profiles
browser-bookmark-sync sync-history --all-profiles --dry-run
# Slower but includes data from all profiles
```

## 📊 Verified Results

```
Test Suite: 8/8 passed ✅

Sync Statistics:
├── Bookmarks: 41,661 URLs, 1,936 folders
├── History: 30,301 unique items
├── Cookies: 925 unique
└── Performance: ~1.1s (release build)
```

## 🔧 Installation

```bash
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync
cargo build --release

# Run tests
cargo test --test integration_test

# Install (optional)
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## ⚠️ Important Notes

1. **Close browsers before syncing** - Prevents database lock errors
2. **Backups are automatic** - Saved to `~/Desktop/browser_backup_*`
3. **Default hubs** - Waterfox + Brave Nightly (customizable with `--browsers`)

## 📜 License

MIT License
