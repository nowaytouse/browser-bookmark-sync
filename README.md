# 🔄 Browser Bookmark Sync

A reliable cross-browser data migration tool using **Hub Browser Architecture**. Migrate bookmarks, history, and reading lists to your primary browsers, then clean up duplicates.

[中文文档](./README_CN.md)

## ✨ Features

- 🎯 **Hub Browser Architecture** - Designate primary browsers, migrate all data to them
- 📚 **Complete Bookmark Migration** - Preserves folder structure, no flattening
- 📜 **Full History Sync** - Merge ALL browsing history (no day limits)
- 📖 **Reading List Migration** - Safari reading list → Hub browser bookmarks
- 🗑️ **Duplicate Cleanup** - Clear non-hub browsers after migration
- 🔒 **Safe Backups** - Automatic backup before every operation
- 🧪 **Tested & Verified** - 7 integration tests included

## 🖥️ Supported Browsers

| Browser | Bookmarks | History | Reading List |
|---------|-----------|---------|--------------|
| **Brave Nightly** | ✅ | ✅ | ✅ (in bookmarks) |
| **Waterfox** | ✅ | ✅ | - |
| **Brave** | ✅ | ✅ | ✅ (in bookmarks) |
| **Chrome** | ✅ | ✅ | ✅ (in bookmarks) |
| **Safari** | ✅ | ✅ | ✅ |
| **Firefox** | ✅ | ✅ | - |

## 🚀 Quick Start

### One-Click Migration (Recommended)

Double-click `sync-now.command` on macOS, or run:

```bash
./sync-now.command
```

### Command Line Usage

```bash
# List all detected browsers
browser-bookmark-sync list

# Validate data integrity
browser-bookmark-sync validate

# Migrate ALL data to hub browsers (recommended)
browser-bookmark-sync migrate \
  --browsers "waterfox,brave-nightly" \
  --history \
  --clear-others

# Preview changes first (dry-run)
browser-bookmark-sync migrate --dry-run

# Scheduled sync (every 30 minutes)
browser-bookmark-sync schedule --cron "0 */30 * * * *"
```

## 📐 Architecture

### Hub Browser Model

```
┌─────────────────────────────────────────────────────┐
│                   HUB BROWSERS                       │
│         Waterfox  ←→  Brave Nightly                 │
│         (Full Data)    (Full Data)                  │
│                                                      │
│  • All bookmarks with folder structure              │
│  • Complete browsing history                        │
│  • Safari reading list (migrated)                   │
└─────────────────────────────────────────────────────┘
                         ↑
              Migrate ALL data, then clear
                         ↑
┌─────────────────────────────────────────────────────┐
│                 NON-HUB BROWSERS                     │
│     Chrome | Brave | Safari | Firefox               │
│     (Cleared after migration)                       │
└─────────────────────────────────────────────────────┘
```

### Migration Rules

1. **Bookmarks**
   - Uses browser with best folder structure as base
   - Preserves complete tree hierarchy
   - URL deduplication

2. **History**
   - Merges ALL history from ALL browsers (no day limit)
   - Deduplicates by URL
   - Sorted by last visit time

3. **Reading Lists**
   - Safari reading list → Hub browser bookmark folder
   - Chromium reading lists are part of bookmarks

## 📊 Verified Results

```
Test Suite: 7/7 passed ✅

Migration Statistics:
├── Waterfox: 24,361 URLs, 1,252 folders
├── Brave Nightly: 41,661 URLs, 1,936 folders  
├── History: 30,301 unique items (merged from 99,114)
└── Space Saved: 156MB (92% reduction)
```

## 🔧 Installation

```bash
# Clone
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync

# Build
cargo build --release

# Test
cargo test --test integration_test

# Install (optional)
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## 🧪 Testing

```bash
# Run all tests
cargo test --test integration_test

# Tests:
# ✅ test_list_command
# ✅ test_validate_command
# ✅ test_migrate_dry_run
# ✅ test_migrate_with_history_dry_run
# ✅ test_migrate_with_clear_others_dry_run
# ✅ test_help_commands
# ✅ test_full_migration_dry_run
```

## ⚠️ Important Notes

1. **Close browsers before migration** - Avoid database locks
2. **Backups are automatic** - Saved to `~/Desktop/browser_backup_*`
3. **Use --dry-run first** - Preview changes before executing

## 📁 Project Structure

```
browser-sync/
├── src/
│   ├── main.rs          # CLI (migrate, validate, list, schedule)
│   ├── browsers.rs      # Browser adapters
│   ├── sync.rs          # Migration engine
│   └── scheduler.rs     # Scheduled tasks
├── tests/
│   └── integration_test.rs  # 7 test cases
├── sync-now.command     # One-click script (macOS)
└── README.md
```

## 📜 License

MIT License
