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

Within one CLI content-check invocation, site preflight and content probing retain the same scheduler, including remaining server `Retry-After` waits and host/domain pacing. Changing phase does not reset the route, request limits or database connection. This is not a machine-wide traffic limiter or a promise that all backend-internal requests already have individual scheduling; see the [network boundaries](docs/health-accuracy.md#network-pacing-across-phases).

Oversized or interrupted HTML is not complete content evidence. A query-bearing canonical route (for example `/index.php?article=42`) is not treated as a lost homepage redirect. Homepage-like redirects without stronger evidence remain review-only, not deletion evidence. YouTube-specific removal checks require an associated player assignment on a supported video URL, not error text quoted in ordinary HTML; unavailable, restricted and conflicting player states stay review-only. Qualifying history requires a real, nonfuture run completion and valid request timestamps no later than that completion; invalid time metadata is never replaced with the current time or silently dropped to manufacture consensus. Real success times are ordered as instants, not timestamp text; an unorderable real success prevents death confirmation rather than disappearing. Evidence reads use one database snapshot rather than mixing a precommit success cutoff with postcommit failures. Rule version 13 and the request-strategy hash prevent reuse of earlier classifications under the current semantics. See [accuracy and reliability boundaries](docs/health-accuracy.md) for reference-tool comparisons and executable acceptance cases.

Checks append process, status, scan lifecycle, and progress events to `browser-sync.log` under `--export-dir` (or to `./browser-sync-check.log` when no directory is supplied). The same directory contains an atomic `check-status.json`: a normal exit records `completed`, while SIGINT/SIGTERM records `aborted` and the signal name is logged. SIGKILL cannot be handled by any process; in that case `running` plus a dead PID and no later `process_finished` event is durable interruption evidence. Use the global `--log-file` option to choose another log path.

Status schema 2 also records the current `phase` (`preparing`, `site`, or `content`) and phase-local `completed_targets` / `total_targets`. Unknown totals are `null`, not zero. Completed targets include reused results and are not a count of new network requests. Progress writes are throttled to once every two seconds, except the first target, phase boundaries, and terminal updates. Counts never regress when concurrent callbacks arrive out of order, and late callbacks cannot overwrite an aborted/completed status. A quiet timestamp alone does not prove a stalled process: requests, retries, and cooldowns may still be in flight. `network_consistency` remains `unverified`; `deletion_stage_status` is `not_run`. Neither 100% phase progress nor a completed process proves source coverage or grants permission to delete.

`check --delete` is intentionally disabled and returns a nonzero exit status. Use the quarantine review, then the separately approved `bsync health apply-quarantine` workflow. No command in the supported surface writes browser profiles.

Useful supporting commands are `collect`, `dedupe`, `list`, `validate`, and `merge-results`. `--all-profiles` is the default for collection/export; `--include` accepts additional bookmark HTML/JSON sources.

Rejected `export` options return a nonzero exit status. Incremental `export --update <existing.html>` reads the existing HTML before inspecting sources and stops if that file cannot be read; it never substitutes an empty collection for an unreadable update input. These failures leave any existing output file untouched.

Optional `export --validate-copy <new.html>` requires a new destination distinct from the export HTML and its report. Path conflicts, existing files (including broken symlinks), missing parent directories, and zero validation concurrency fail before source collection. Validation uses a private staged copy and publishes it without overwriting another file only after parsing and checking succeed. Copy, parser, checker initialization, and report errors return failure; a copy already published before a later report-write error is retained, but the command still fails. Incremental validation writes a fresh validation-only report, not old export statistics. URL-limited validation sorts targets before selecting the limit.

HTML imports reject ordinary text and input whose bookmark entries cannot be parsed completely without changing URLs/titles or losing entries. An explicitly empty bookmark list remains valid. Some nonstandard HTML layouts/entities are intentionally rejected with a re-export instruction rather than accepted as a partial import. Dates outside the representable millisecond range are left unknown, not replaced with the current time.

`merge-results` requires at least one result partition (`valid.html`, `invalid.html`, `uncertain.html`, or `skipped.html`). Every present partition must be readable and parse successfully before an output is written; an output path that resolves to a partition input is rejected. Absent partitions remain optional.

Health retry, quarantine selection, and summary rebuild fail when the requested database is missing. Retry validates all selected URLs before starting probes, instead of silently dropping invalid rows. Dashboard watch/monitor commands surface database errors and return failure; a monitor waiting for a not-yet-created database still waits, and its refresh delay can be interrupted with Ctrl+C/SIGTERM.

## Template organization

Automatic AI organization is not part of the current product. If a future release exposes template organization, it must be an explicit file-only operation with a required rules/template file; it must never be part of export or health checking by default.

## Native desktop review GUI

```bash
cargo run --features native-gui --bin bookmark-review -- \
  --input ./bookmarks-cleaned.html \
  --db ./bookmark-health.sqlite --source-review ./source-review.json
```

The Rust desktop window uses egui/eframe, not HTML, JavaScript, a WebView, or a local server. It runs independently of ChatGPT. Start without arguments to import an HTML file (including drag and drop) or explicitly import all discovered browser Profiles and supported reading lists from read-only snapshots. An expandable source block lists browser, Profile, collection, read state and original record count. Verified empty, unavailable, unsupported and failed sources are distinct; source counts do not imply universal coverage. `--db` and `--source-review` are optional; missing health evidence is untested, never dead.

The single-purpose workspace shows import, duplicate, health and export counts without a navigation sidebar. Bookmarks can be grouped by full folder path, website or check status, or shown in a flat list. Groups are collapsible and have partial/all selection controls. The selection menu selects or inverts the current filtered set, selects untested/uncertain entries, or clears selection. Grouping never moves bookmarks, changes exported folders or merges occurrence IDs. Later occurrences of an exact full URL are marked for removal after import; the first occurrence is retained. Deduplication can be undone, and every occurrence remains independently selectable. Search/filter results, inspect evidence in the optional inspector, and mark individual or selected groups **keep** or **remove**. Confirm the exact removal count before saving a new HTML copy plus audit. Export checks the original file hash and retained bookmark/folder metadata, and refuses to overwrite existing output. The original HTML and real browser profiles remain untouched.

Settings (`⌘,` on macOS) provide Appearance, Import & evidence, Network & checks, Advanced and About tabs. Appearance includes system/light/dark themes, blue/teal/violet/monochrome accents, system/English/Simplified Chinese language, list density, interface scale, system/reading fonts and balanced/list-first/details-first width proportions. The default grouping can be saved independently of temporary list changes. On macOS the system face uses SF with a PingFang fallback; hosts without those fonts use bundled faces. Display changes can be previewed; **Apply** saves the complete draft and **Cancel** or Escape discards it. Only display and non-secret connection preferences are saved locally; bookmark contents and source paths are not saved in the preferences file. Developer mode is off by default; only when enabled do raw per-bookmark ID/verdict/decision suffixes and footer flags appear. It is display-only, never a safety bypass. About shows the theme-aware bookmark mark, version and public-source link; the repository currently has no declared license file. `⌘O`, `⌘S`, and `⌘Z` import, export and undo.

**Try sample bookmarks** on the welcome screen loads a private copy of 20 synthetic entries. It lets beginners practice groups, selection, Undo and export without reading browser data. The fixture has two exact duplicate occurrences, nested Chinese folders, long/empty titles and non-web links. No network check runs automatically, and all sample links remain untested.

On macOS, **Start check** prepares a private, fresh input copy and database, then uses the existing launchd supervisor. The scan is independent of both this window and ChatGPT, with no overall deadline. The App bundle must include its `bsync` engine and Python 3 is required. Network settings offer a local HTTP proxy (default `http://127.0.0.1:6152`, configurable host/port) or explicit direct routing that does not inherit environment proxies, plus a 5–60 second request timeout. **Check proxy port** only tests a local TCP listener; success is not proof of working HTTP proxying or internet access. The chosen timeout, route and hashed DNS policy are snapshotted into each batch and revalidated before its worker starts. Applying new preferences never reconfigures an existing scan.

Direct checks offer automatic DNS evidence routing (AliDNS for Chinese domains, Cloudflare otherwise), Cloudflare-first or AliDNS-first, and an optional backup path on temporary primary failure. Cloudflare falls back to AliDNS and vice versa. These are encrypted DoH diagnostics for failed sites, not replacement DNS for HTTP requests or changes to system DNS. NXDOMAIN confirmation always keeps two distinct providers, even if error fallback is disabled. One temporary failure followed by one NXDOMAIN is not two-provider death confirmation. Old external policy JSON defaults to no temporary-error fallback; GUI defaults enable it for new batches only. Proxy mode does not run independent direct DNS probes; the UI makes the saved DNS choice inactive until direct mode is selected. Slow mode, concurrency 2, curl confirmation and full target coverage remain fixed conservative defaults.

**Refresh progress** updates phase-local counters only when you ask; **Load check results** preserves current review decisions and checks planned Site/Content coverage before claiming completion. Use **Open check batch** to reopen a saved batch after quitting. No periodic progress monitor is created or resumed. Browser source coverage is separate from network target completion.

Health results are historical evidence, not a live guarantee; results older than 24 hours are flagged. Manual removal of uncertain entries is your explicit choice, not automatic confirmation that they are dead. Source-coverage gaps remain visible even when all supplied URLs have been checked. The native toolkit supports desktop platforms, but platform acceptance must be tested separately; browser-source discovery is currently macOS-oriented. The legacy server GUI stays disabled.

## Safety boundaries

- No browser-profile write-back or cloud sync.
- No automatic destructive deletion from a single check.
- No password, cookie, history, download, or extension export.
- Quarantine is a review boundary; only a separately approved apply step may remove entries from a generated copy.
- Original browser/profile data is left in place for manual cleanup after review.

## License

MIT License © 2026 Browser Sync Team
