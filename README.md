# 🔄 Browser Bookmark Sync

A reliable cross-browser bookmark, history, and reading list synchronization tool. Uses a **Hub Browser Architecture** to prevent data duplication and maintain organization.

[中文文档](./README_CN.md)

## ✨ Features

- 🎯 **Hub Browser Mode** - Designate primary browsers, automatically clean others
- � **Bookm步ark Sync** - Preserves complete folder structure, no flattening
- 📜 **History Sync** - Merge browsing history across browsers with deduplication
- 📖 **Reading List Migration** - Safari reading list → Hub browser bookmarks
- 🍪 **Cookie Sync** - Cross-browser cookie migration
- ⏰ **Scheduled Sync** - Cron expression support for automatic syncing
- � ️**Safe Backups** - Automatic backup before every operation
- 🧪 **Tested & Verified** - Integration test suite included

## 🖥️ Supported Browsers

| Browser | Bookmarks | History | Reading List | Cookies |
|---------|-----------|---------|--------------|---------|
| **Brave Nightly** | ✅ | ✅ | - | ✅ |
| **Waterfox** | ✅ | ✅ | - | ✅ |
| **Brave** | ✅ | ✅ | - | ✅ |
| **Chrome** | ✅ | ✅ | - | ✅ |
| **Safari** | ✅ | ✅ | ✅ | - |
| **Firefox** | ✅ | ✅ | - | ✅ |
| **LibreWolf** | ✅ | ✅ | - | ✅ |

## 🚀 Quick Start

### One-Click Sync (Recommended)

Double-click `sync-now.command` on macOS:

```bash
# Or run in terminal
./sync-now.command
```

This will automatically:
1. Backup current data to Desktop
2. Sync Brave Nightly ↔ Waterfox bookmarks and history
3. Migrate Safari reading list to hub browsers
4. Clean duplicate data from non-hub browsers

### Command Line Usage

```bash
# List all detected browsers
browser-bookmark-sync list

# Validate bookmark integrity
browser-bookmark-sync validate

# Set hub browsers and sync (recommended)
browser-bookmark-sync set-hubs \
  --browsers "waterfox,brave-nightly" \
  --sync-history \
  --clear-others

# Preview changes without executing
browser-bookmark-sync set-hubs --dry-run

# Sync bookmarks only (all browsers)
browser-bookmark-sync sync

# Sync history (last 30 days)
browser-bookmark-sync sync-history --days 30

# Scheduled sync (every 30 minutes)
browser-bookmark-sync schedule --cron "0 */30 * * * *"
```

## 📐 Sync Architecture

### Hub Browser Model

```
┌─────────────────────────────────────────────────────┐
│                   HUB BROWSERS                       │
│         Waterfox  ←→  Brave Nightly                 │
│         (Full Data)    (Full Data)                  │
└─────────────────────────────────────────────────────┘
                         ↑
                  Migrate & Clear
                         ↑
┌─────────────────────────────────────────────────────┐
│                 NON-HUB BROWSERS                     │
│     Chrome | Brave | Safari | LibreWolf             │
│     (Cleared) (Cleared) (Cleared) (Cleared)         │
└─────────────────────────────────────────────────────┘
```

### Sync Rules

1. **Bookmarks**
   - Uses browser with best folder structure as base
   - Preserves complete tree hierarchy (no flattening)
   - URL deduplication (same URL kept once)

2. **History**
   - Merges history from all browsers
   - Deduplicates by URL
   - Sorted by last visit time

3. **Profile Handling**
   - Only syncs Default Profile
   - Cleans duplicate data from other profiles

## 📊 Verified Test Results

```
Test Suite: 6/6 passed ✅

Data Statistics:
├── Waterfox: 24,361 URLs, 1,252 folders
├── Brave Nightly: 41,661 URLs, 1,936 folders  
├── History: 30,301 unique items (merged)
└── Space Saved: 156MB (92% reduction)
```

## 🔧 Installation

```bash
# Clone repository
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync

# Build
cargo build --release

# Run tests
cargo test --test integration_test

# Install to system (optional)
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## 🧪 Testing

Run the integration test suite:

```bash
cargo test --test integration_test
```

Tests cover:
- ✅ Browser detection (`list`)
- ✅ Data validation (`validate`)
- ✅ Bookmark sync (`sync`)
- ✅ History sync (`sync-history`)
- ✅ Hub configuration (`set-hubs`)
- ✅ Help commands

## ⚠️ Known Limitations

1. **Browser Running** - Close browsers before syncing to avoid database locks
2. **Safari Reading List Write** - Read-only (migrates to bookmark folder instead)
3. **Multi-Profile** - Only syncs Default Profile to prevent duplication

## 📁 Project Structure

```
browser-sync/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── browsers.rs      # Browser adapters
│   ├── sync.rs          # Sync engine
│   ├── scheduler.rs     # Scheduled tasks
│   └── validator.rs     # Data validation
├── tests/
│   └── integration_test.rs  # Test suite
├── sync-now.command     # One-click sync (macOS)
├── empty_bookmarks.json # Empty bookmark template
└── README.md
```

## 📜 License

MIT License
