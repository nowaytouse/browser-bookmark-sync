# Native review handoff — 2026-10-01

The optional `bookmark-review` binary is a Rust/egui desktop application, not a web page or WebView. It only reads exported HTML copies and optional health/source evidence. Collection and network checking remain separate CLI operations.

## Use

```sh
cargo run --features native-gui --bin bookmark-review -- \
  --input /path/to/export.html \
  --db /path/to/bookmark-health.sqlite \
  --source-review /path/to/source-review.json
```

All arguments are optional. Without them, use the native file chooser. Select optional evidence files before opening the HTML; changing these selections applies to the next load, not the current document.

Search title, URL or folder path; filter by historical verdict or manual decision. Checkbox selection is separate from keep/remove decisions. “仅全选当前筛选结果” replaces the selection with every matching occurrence, including rows outside the visible viewport. Hidden selections are counted explicitly. Individual duplicate URL occurrences can be reviewed separately. Undo reverses the last decision action.

“另存审阅副本” asks for a new destination, then confirms retained and removed counts. It creates the HTML and a sibling `*.review-audit.json`; it never replaces the original or an existing destination. No database means untested, not dead. No entry is automatically marked for removal. Manual removal is not presented as a health verdict.

The model checks input hashes, retained anchors, folder headings and order before publishing. Temporary output is private on Unix (0600); output publication refuses clobbering. The health database is opened read-only and only completed content-check evidence is attached to exact URLs.

## Verification and remaining acceptance

- Full native-feature test run: 402 tests across 11 suites passed; Clippy all targets with `-D warnings` passed.
- Regression coverage includes duplicate occurrences, nested Chinese folders, metadata, unchanged input, stale input, invalid decision IDs, output/audit collisions, private output permissions, read-only database evidence, filtered bulk decisions and undo.
- macOS optimized build completed and the packaged application opened. Chinese text and the accessibility tree were present. A five-entry fixture loaded and marking only duplicate A for removal changed the count from 0 to 1, leaving duplicate B undecided.
- A subsequent native UI pass verified single removal, undo, search, selecting both filtered duplicates, bulk removal and explicit native-dialog export. The actual saved fixture contains three retained anchors; its audit records five source entries, two removals (duplicate A and B), and three retained entries. The original tracked fixture is unchanged. Evidence is in ignored `target/gui-acceptance-20261001.html` and its sibling review audit.
- The fresh 23,742-entry real export and its source audit loaded in the native window. Searching `github` produced 4,547 matches; selecting all matched entries, scrolling three pages, clearing selection/search, and cancelling the native Save dialog worked. The final window retained all 23,742 entries with zero selection/removal decisions and visibly reported partial source coverage. Input SHA-256 stayed unchanged. This is a functional UI check, not a frame-time benchmark.
- Windows/Linux builds and UI have not been tested. The toolkit is cross-platform, but browser collection remains macOS-oriented.
- Universal source coverage is still incomplete: Safari custom Favorites/profile mapping, independent Safari Technology Preview bookmark storage, and Gecko reading-list support remain unverified. No GUI success closes these source gaps.

No live browser profile was modified. At the user's renewed request, a fresh independent scan was started at 2026-10-01 20:50:10 +08 from `bookmark-health-run/clean-20261001-aMMCpA`, using a new database and newly exported 23,742 URLs. The immutable batch launcher is managed by launchd, not the chat client's process tree. The initial read-only snapshot contained 80 attempts, 76 real requests, no cache hits, 55 HTTP 200 responses and 74 saved targets; these are startup figures, not completion evidence. Both site and content coverage still require final verification.

The launcher and its three mocked safety tests are in `scripts/`. The permanent per-batch request/manifest and parent-directory file lock prevent duplicate starts. Successful process exit is explicitly pending coverage verification, not task completion. Never relaunch this batch or delete its request/lock.

## Remaining acceptance

1. Exercise dirty-document discard/cancel, save success followed by close, and existing-output rejection through the GUI (model guards already have regression coverage).
2. Verify final site/content plan coverage when the new scan exits, then create a new cleaned HTML copy from qualified dead-link evidence; do not infer death from untested or transient failures.
3. Resolve or explicitly document the remaining browser-source gaps and perform platform-specific Windows/Linux acceptance before claiming those platforms verified.
