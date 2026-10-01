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
- Full native-dialog export and large real-export UI acceptance are **not yet verified**. During the subsequent pass, the computer-use tool repeatedly rejected actions with “The user changed … Re-query the latest state”. No guard was bypassed. Backend export tests are not claimed as a complete GUI acceptance pass.
- Windows/Linux builds and UI have not been tested. The toolkit is cross-platform, but browser collection remains macOS-oriented.
- Universal source coverage is still incomplete: Safari custom Favorites/profile mapping, independent Safari Technology Preview bookmark storage, and Gecko reading-list support remain unverified. No GUI success closes these source gaps.

No live browser profile was modified; no new network scan was started during this GUI work. The existing five-hour automation was found paused and left paused.

## Next bounded acceptance pass

1. Load `tests/fixtures/review-bookmarks.html` in the desktop app.
2. Mark one duplicate, undo it, search `duplicate`, select the two results, mark both and undo.
3. Mark the intended test entries, save to a fresh test destination, confirm the displayed count, and reparse the new HTML and audit; verify the original is unchanged.
4. Exercise discard/cancel, save success followed by close, and existing-output rejection.
5. Load the real exported copy with its source audit, verify responsive scrolling/filtering, and report its actual evidence coverage without deleting untested entries.
