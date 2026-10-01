# Browser Bookmark Cleanup (bsync)

`bsync` is a macOS, pure-Rust utility for one job: collect bookmarks from every supported browser and profile, make a deterministic clean copy, and identify stale site or page links for review.

It does not synchronize browsers, write browser profiles, upload bookmarks to cloud services, run a web server, export passwords/cookies/history, or call a local AI model. The original browser data remains untouched; import a reviewed HTML copy yourself.

## Supported workflow

1. Discover all browser installations and profiles.
2. Collect a complete bookmark tree and exact URL deduplication evidence.
3. Export a cleaned HTML copy with duplicate and empty-folder cleanup.
4. Run same-day site and content health checks with bounded retries.
5. Review quarantine candidates before any separate, explicitly approved apply step.

The collector covers Chromium-family profiles, Gecko profiles, and Safari where their local bookmark stores are available. A failed profile is reported instead of silently producing a partial harvest.

Add `--reading-list` to export Safari and Chromium reading lists from isolated snapshots. Chromium local and account records are included; per-source counts and unsupported Gecko reading-list coverage appear in the export report. An unreadable or changing store fails the export instead of masquerading as an empty list. Safari Technology Preview bookmark stores are discovered separately when present. Safari custom profile-to-Favorites mappings are not yet verified, so these reports do not claim universal source completeness.

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

# Validate the exported copy; --slow limits per-host request load
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

The trusted-site list skips only redundant site preflight work; it never skips page, repository, post, or video checks. Content probing is suppressed only when the site itself is already `ConfirmedDead`. Raw 404/410 responses require three completed, uncached same-day runs and two verifier families before a content URL becomes a quarantine candidate. The current run must also contain qualifying evidence; interrupted transfers cannot borrow older failures to confirm a deletion candidate. A real successful retry keeps the bookmark.

The default secondary verifier is `curl`. Every secondary request re-enters the same host/domain rate limiter and shares its cooldowns. Slow mode spaces scheduled primary/secondary probes to the same host/domain by at least two seconds and allows at most one configured secondary verifier per strong-failure round. Without a working second verifier, raw 404/410 responses remain unconfirmed. Unauthenticated probes cannot reliably distinguish a deleted resource from a private resource that deliberately returns an ordinary 404, so quarantine still requires human review.

Checks append process, status, scan lifecycle, and progress events to `browser-sync.log` under `--export-dir` (or to `./browser-sync-check.log` when no directory is supplied). The same directory contains an atomic `check-status.json`: a normal exit records `completed`, while SIGINT/SIGTERM records `aborted` and the signal name is logged. SIGKILL cannot be handled by any process; in that case `running` plus a dead PID and no later `process_finished` event is durable interruption evidence. Use the global `--log-file` option to choose another log path.

Status schema 2 also records the current `phase` (`preparing`, `site`, or `content`) and phase-local `completed_targets` / `total_targets`. Unknown totals are `null`, not zero. Completed targets include reused results and are not a count of new network requests. Progress writes are throttled to once every two seconds, except the first target, phase boundaries, and terminal updates. Counts never regress when concurrent callbacks arrive out of order, and late callbacks cannot overwrite an aborted/completed status. A quiet timestamp alone does not prove a stalled process: requests, retries, and cooldowns may still be in flight. `network_consistency` remains `unverified`; `deletion_stage_status` is `not_run`. Neither 100% phase progress nor a completed process proves source coverage or grants permission to delete.

`check --delete` is intentionally disabled. Use the quarantine review, then the separately approved `bsync health apply-quarantine` workflow. No command in the supported surface writes browser profiles.

Useful supporting commands are `collect`, `dedupe`, `list`, `validate`, and `merge-results`. `--all-profiles` is the default for collection/export; `--include` accepts additional bookmark HTML/JSON sources.

## Template organization

Automatic AI organization is not part of the current product. If a future release exposes template organization, it must be an explicit file-only operation with a required rules/template file; it must never be part of export or health checking by default.

## Native desktop review GUI

```bash
cargo run --features native-gui --bin bookmark-review -- \
  --input ./bookmarks-cleaned.html \
  --db ./bookmark-health.sqlite --source-review ./source-review.json
```

The Rust desktop window uses egui/eframe, not HTML, JavaScript, a WebView, or a local server. It runs independently of ChatGPT. Start without arguments to open an HTML copy using the native file chooser. `--db` and `--source-review` are optional; missing health evidence is untested, never dead. This window reviews exported files; it does not silently start collection or network scanning.

Search and filter results, inspect historical evidence, open an HTTP(S) address yourself, and mark individual or selected groups **keep** or **remove**. Nothing is preselected for removal. Undo or reset decisions before confirming the exact removal count and saving a new HTML copy plus audit. Each duplicate URL occurrence is independently selectable. Export checks the original file hash and retained bookmark/folder metadata, and refuses to overwrite existing output. The original HTML and real browser profiles remain untouched.

Health results are historical evidence, not a live guarantee; results older than 24 hours are flagged. Manual removal of uncertain entries is your explicit choice, not automatic confirmation that they are dead. Source-coverage gaps remain visible even when all supplied URLs have been checked. The native toolkit supports desktop platforms, but platform acceptance must be tested separately; browser-source discovery is currently macOS-oriented. The legacy server GUI stays disabled.

## Safety boundaries

- No browser-profile write-back or cloud sync.
- No automatic destructive deletion from a single check.
- No password, cookie, history, download, or extension export.
- Quarantine is a review boundary; only a separately approved apply step may remove entries from a generated copy.
- Original browser/profile data is left in place for manual cleanup after review.

## License

MIT License © 2026 Browser Sync Team
