# 🔖 Browser Bookmark Sync (bsync)

A fast, cross-browser bookmark management tool for macOS. Merge, deduplicate, and export bookmarks from multiple browsers.

## ✨ Features

- **Multi-Browser**: Safari, Chrome, Brave, Brave Nightly, Waterfox, Firefox
- **HTML Export**: Standard Netscape format (importable everywhere)
- **Smart Deduplication**: Remove duplicates across all sources
- **Auto-Classification**: 48 built-in rules to organize bookmarks
- **Safari Reading List**: Export reading list as bookmarks
- **Safe by Default**: Export-only, no browser modifications

## 🚀 Quick Start

```bash
# Build
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync

# Basic usage
bsync list                              # List browsers
bsync export -d --merge                 # Export all, deduplicated
bsync export -b safari -r               # Safari + reading list
bsync analyze                           # Check for issues
```

## 📖 Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `list` | `l` | List detected browsers |
| `export` | `e` | Export bookmarks to HTML |
| `analyze` | `a` | Analyze bookmarks |
| `organize` | `o` | Smart organize by URL |
| `validate` | `v` | Validate integrity |
| `history` | `hist` | Sync browser history |
| `rules` | - | Show classification rules |
| `backup` | - | Create full backup |

## 📤 Export Command

The main command for bookmark management:

```bash
bsync export [OPTIONS]
```

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--output <FILE>` | `-o` | Output path (default: ~/Desktop/bookmarks.html) |
| `--browsers <LIST>` | `-b` | Source browsers (default: all) |
| `--deduplicate` | `-d` | Remove duplicates |
| `--merge` | `-m` | Flat structure (no browser folders) |
| `--clean` | - | Remove empty folders |
| `--reading-list` | `-r` | Include Safari reading list |
| `--include <FILE>` | - | Import existing HTML |
| `--clear-after` | - | Clear sources after export (⚠️) |
| `--verbose` | `-v` | Detailed output |

### Examples

```bash
# Export all browsers, deduplicated, merged
bsync export -d -m -o ~/bookmarks.html

# Safari only with reading list
bsync export -b safari -r -d

# Merge multiple sources
bsync export -b "safari,brave" -d -m --include old_backup.html

# Full cleanup export
bsync export -d -m --clean
```

## 🧠 Smart Organization

Auto-classify bookmarks into 48 categories:

```bash
# Preview (safe)
bsync organize --dry-run --stats

# Apply to specific browser
bsync organize -b safari

# Custom rules
bsync organize -r my-rules.json
```

### Categories

- 🎬 Streaming, Video
- 🎮 Gaming
- 💻 Development, GitHub
- 📱 Social Media
- 🛒 Shopping
- 📰 News, Blogs
- And 40+ more...

## 🔍 Analysis

Check bookmarks for issues:

```bash
bsync analyze
bsync analyze -b safari
```

Detects:
- Duplicate URLs
- Empty folders
- NSFW content (stats only)

## 🌐 Supported Browsers

| Browser | Bookmarks | History | Reading List |
|---------|-----------|---------|--------------|
| Safari | ✅ | ✅ | ✅ |
| Chrome | ✅ | ✅ | - |
| Brave | ✅ | ✅ | - |
| Brave Nightly | ✅ | ✅ | - |
| Waterfox | ✅ | ✅ | - |
| Firefox | ✅ | ✅ | - |

## ⚠️ Important Notes

1. **Close browsers** before operations
2. **Export is safe** - doesn't modify browser data
3. **--clear-after is destructive** - use with caution
4. **Browser sync conflicts** - if sync is enabled, manual import is safer

## 📊 Example Output

```
📤 Exporting bookmarks to HTML
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Output: ~/Desktop/bookmarks.html
Source: all
  ✓ Deduplicate
  ✓ Merge (flat)
  ✓ Include Safari reading list
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🎯 Target browsers:
  - Safari
  - Brave Nightly
  - Waterfox
📖 Reading Safari reading list...
   42 items found
📊 Collection complete: 178326 bookmarks
🧹 Deduplicating...
  ✅ Removed 154805 duplicate bookmarks

✅ Exported 23521 bookmarks to ~/Desktop/bookmarks.html
```

## 📄 License

MIT License
