# Native Bookmark Cleaner handoff — 2026-10-12

`bookmark-review` is a Rust/egui desktop application, not a web page or WebView. The main window follows one workflow: import, exact deduplication, check/review, then export a new consolidated HTML. There is no navigation sidebar.

## GUI workflow

1. Open **Import HTML** or **Import browsers**. Browser import reads temporary copies and shows browser, Profile, collection, state and raw count. Missing, unsupported, unverified and successfully read empty sources are different states. A failed refresh keeps the previously loaded document.
2. Exact duplicate URLs are initially marked for removal after their first occurrence. These are copy-cleanup decisions, not dead-link verdicts, and can be reset or undone. Uncertain and untested URLs remain retained unless the user explicitly chooses removal.
3. On macOS, **Start check** creates a private fresh batch with its own input, engine and empty database. It uses the existing launchd supervisor, independently of the App and ChatGPT, without an overall deadline. Python 3 and a reachable local HTTP proxy are required; the bundled App includes the engine. The default session proxy is `http://127.0.0.1:6152` and can be changed in Settings.
4. **Refresh progress** is manual and displays the last reported batch state, not a fresh process-liveness check. There is no agent progress monitor or automatic polling loop. **Load check results** attaches read-only evidence while preserving review decisions. **Open check batch** reopens a batch after closing the App. Successful process exit is not accepted as complete until the immutable input and all planned site/content targets reconcile with completed database runs.
5. Search title, URL or folder; filter by verdict or decision. Checkbox selection is separate from keep/remove decisions. Select all filtered results includes rows outside the visible viewport, with hidden selections counted. Review evidence in the optional inspector, decide individually or in bulk, and use Undo as needed.
6. **Export HTML** asks for a new destination and confirms retained/removal counts. It publishes new HTML plus a sibling `*.review-audit.json`, refusing to overwrite the input or any existing output. The original HTML and live browser Profiles are untouched.

Settings (`⌘,`) provide system/light/dark appearance, blue/teal/monochrome accents, system/English/Simplified Chinese language, list density, interface scale and inspector visibility. Display changes can be previewed, applied or cancelled. Only display preferences are persisted; bookmark contents and paths are not saved in that file. About and optional existing evidence attachments share the settings window. `⌘O`, `⌘S` and `⌘Z` import, export and undo.

## Safety and verification

The export model verifies input hashes, retained anchors, folder headings, metadata and order before publishing. Private output uses Unix permissions 0600 and no-clobber publication. Evidence is read through a read-only SQLite connection; only completed content-check evidence is attached to exact URLs. A manual removal is never represented as a health verdict.

The current validation covers 428 tests across 11 suites, launcher mock tests, formatting, and all-target Clippy with warnings denied. Regression cases include duplicate occurrences, Unicode folders, metadata, stale input, invalid decisions, output/audit collisions, private permissions, source-state semantics, immutable batch input, complete target coverage and evidence reload preserving decisions.

Native macOS UI acceptance exercised settings/theme/language previews, search, filtered selection, bulk removal, Undo, inspector and native-dialog export. The five-entry fixture exported four retained links and one exact-duplicate removal, retaining uncertain/non-web links and metadata. The original tracked fixture stayed unchanged. Evidence is under ignored `target/native-gui-20261010/test-evidence/`.

The delivered App is a development build with local ad-hoc signing, not a notarized release. Its bundled engine is verified against the build output. A new real network scan was not started during this GUI handoff; the user controls subsequent runs from the App. The earlier `clean-20261001-aMMCpA` batch is historical evidence, not a current running task.

## Known boundaries

- Universal source coverage is not verified: Safari custom Favorites/Profile mappings, independent Safari Technology Preview storage and Gecko reading-list support remain gaps. Successful import of discovered sources or completion of their network check cannot close these gaps.
- Windows/Linux builds and UI have not been accepted. The toolkit is cross-platform, but independent launch and browser collection in this handoff remain macOS-oriented.
- The new GUI scan-start path has regression and mock coverage, but a fresh live network run was intentionally left for the user. Historical evidence is not current network truth.
- Existing-output rejection and dirty-document guards have model tests; they are not claimed as a complete native-dialog acceptance matrix.
