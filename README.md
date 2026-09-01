# Browser Bookmark Cleanup (bsync)

`bsync` is a macOS, pure-Rust utility for one job: collect bookmarks from every supported browser and profile, make a deterministic clean copy, and identify stale site or page links for review.

It does not synchronize browsers, write browser profiles, upload bookmarks to cloud services, run a Web GUI, export passwords/cookies/history, or call a local AI model. The original browser data remains untouched; import a reviewed HTML copy yourself.

## Supported workflow

1. Discover all browser installations and profiles.
2. Collect a complete bookmark tree and exact URL deduplication evidence.
3. Export a cleaned HTML copy with duplicate and empty-folder cleanup.
4. Run same-day site and content health checks with bounded retries.
5. Review quarantine candidates before any separate, explicitly approved apply step.

The collector covers Chromium-family profiles, Gecko profiles, and Safari where their local bookmark stores are available. A failed profile is reported instead of silently producing a partial harvest.

## Install

```bash
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync
```

## Core commands

```bash
# Inspect browsers and every discovered profile
bsync inventory

# Export a deterministic, deduplicated bookmark copy
bsync export -o ~/Downloads/all_bookmarks.html --all-profiles -d --clean

# Validate the exported copy; --slow enables same-day historical evidence
bsync check --file ~/Downloads/all_bookmarks.html \
  --slow --history-db ./bookmark-health.sqlite \
  --export-dir ~/Downloads/check_results

# Review candidates and write a selection file; review is non-destructive
bsync health quarantine --db ./bookmark-health.sqlite \
  --output ./quarantine-selection.json

# Only after review: filter a generated check copy into a new output file
bsync health apply-quarantine --db ./bookmark-health.sqlite \
  --selection ./quarantine-selection.json \
  --input ~/Downloads/all_bookmarks.html \
  --output ~/Downloads/bookmarks-cleaned.html --confirm
```

`check` has two distinct validation targets: site availability and the saved page/video/resource path. A temporary failure is retried within the same day. A real, uncached success preserves the bookmark as live/recovered even when another probe disagrees. Uncertain results remain for review.

Checks append lifecycle and progress events to `browser-sync.log` under `--export-dir` (or to `./browser-sync-check.log` when no directory is supplied). The same directory contains an atomic `check-status.json`: `running` remains visible after a forced termination, while a normal exit records `completed`; use the global `--log-file` option to choose another log path.

`check --delete` is intentionally disabled. Use the quarantine review, then the separately approved `bsync health apply-quarantine` workflow. No command in the supported surface writes browser profiles.

Useful supporting commands are `collect`, `dedupe`, `list`, `validate`, and `merge-results`. `--all-profiles` is the default for collection/export; `--include` accepts additional bookmark HTML/JSON sources.

## Template organization

Automatic AI organization is not part of the current product. If a future release exposes template organization, it must be an explicit file-only operation with a required rules/template file; it must never be part of export or health checking by default.

## Deferred GUI

The GUI is deferred. The current product has no supported Web GUI and no browser-based dashboard. A future GUI may be added as a separate native implementation; it will not be a web server.

## Safety boundaries

- No browser-profile write-back or cloud sync.
- No automatic destructive deletion from a single check.
- No password, cookie, history, download, or extension export.
- Quarantine is a review boundary; only a separately approved apply step may remove entries from a generated copy.
- Original browser/profile data is left in place for manual cleanup after review.

## License

MIT License © 2026 Browser Sync Team
