# Browser Bookmark Sync (bsync)

Cross-browser bookmark management tool for macOS.

## Features

- Multi-browser support: Safari, Chrome, Brave, Waterfox, Firefox
- HTML export (Netscape format)
- Smart deduplication
- Auto-classification (48 rules)
- Safari Reading List export

## Quick Start

```bash
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync

bsync list                    # List browsers
bsync export -d --merge       # Export all, deduplicated
bsync export -b safari -r     # Safari + reading list
```

## Commands

| Command | Description |
|---------|-------------|
| `list` | List detected browsers |
| `export` | Export bookmarks to HTML |
| `analyze` | Check for issues |
| `organize` | Smart organize by URL |

## Export Options

```bash
bsync export [OPTIONS]

-o, --output <FILE>    Output path
-b, --browsers <LIST>  Source browsers
-d, --deduplicate      Remove duplicates
-m, --merge            Flat structure
-r, --reading-list     Include Safari reading list
-f, --folder <NAME>    Only export specific folder
```

---

MIT License
