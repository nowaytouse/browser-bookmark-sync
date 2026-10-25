# Health accuracy and reliability boundaries

This tool cleans an exported HTML copy; it does not write live browser Profiles. A failed request is not proof that a bookmark is dead. Source collection completeness, transport reachability, content validity and a user's removal decision are separate facts.

## Reference tools and decisions

The 2026-10-25 review compared primary documentation/source, without running external scanners against personal bookmarks:

- [LinkChecker HTTP handling](https://github.com/linkchecker/linkchecker/blob/master/linkcheck/checker/httpurl.py): redirects are recorded separately from final response handling, rate limiting is a distinct warning, and reads above its download limit fail explicitly. We retain bounded reads but no longer treat an HTML prefix as a fully checked page. We do not copy its TLS-verification exception: TLS failures remain non-destructive uncertainty here.
- [lychee website checking](https://github.com/lycheeverse/lychee/blob/master/lychee-lib/src/checker/website.rs) and [rate-limit guidance](https://lychee.cli.rs/troubleshooting/rate-limits/): bounded retries, method fallback, redirect reporting and host backoff are distinct concerns. This project already separates site HEAD/fallback from content GET, and shares host cooldowns with secondary probes. A checker's unsuccessful status is not imported as permission to remove a bookmark.
- [curl configuration isolation](https://curl.se/docs/manpage.html#-q) and [literal URLs](https://curl.se/docs/manpage.html#-g): the secondary command now puts `-q` first, disables URL glob expansion, and supplies one literal `--url`. A user's curl configuration must not add uploads, credentials or additional targets to a bookmark check.

## Acceptance cases

| Input or evidence | Required behavior | Executable check |
| --- | --- | --- |
| 256 KiB complete HTML, with and without Content-Length | Observe EOF and allow ordinary content analysis | `accuracy_incomplete_html_is_not_full_content_evidence` |
| Larger HTML with 200/404/410, with and without Content-Length | `BodyTooLarge`, request completeness unknown, no full body hash/preview | same native backend test |
| Truncated declared-length 404, including a prefix exactly at the cap | Body-read error; no full body hash | same native backend test |
| Article redirect to `/index.php?article=42`, valid final HTML | Keep query-routed content valid | `accuracy_query_routed_redirect_preserves_the_article` |
| Homepage-like redirect plus repeated failure history | Review-only; not strong history evidence | `homepage_redirects_remain_review_only_despite_repeated_history`, `accuracy_heuristic_warnings_are_not_strong_history_evidence` |
| Review loads an older-rule database | Show review-needed with rule-mismatch explanation; preserve raw evidence and DB bytes | `accuracy_review_downgrades_stale_rules_without_changing_the_db` |
| Incomplete, cached, synthetic or transport-failed 404/410 plus three earlier failures | Cannot borrow history to become dead | `accuracy_incomplete_current_failures_cannot_borrow_confirmed_history` |
| Complete 404/410 plus qualifying current-day history | Existing confirmation path remains functional | same state-machine test and engine/database regressions |
| Redirects, access restrictions, rate limiting, 408 and 5xx | No death confirmation solely from those statuses | same state-machine matrix |
| URL containing brackets/braces and ambient curl configuration | Send the literal bookmark once; ignore ambient config | `curl_sends_brackets_and_braces_without_expanding_the_bookmark`, `curl_does_not_read_an_ambient_config_file` |
| Occupied output/audit, including dangling symlinks or entries appearing after preflight | Fail without overwriting the entry | `quarantine_no_clobber_*` |

Rule version 9 and request strategy v4 invalidate old positive reuse and old qualifying history for the new semantics. Historical evidence is retained, not silently relabeled. The engine also independently requires qualifying evidence from the current completed run before confirming death. The state-machine guard is defense in depth, not a claim that the previous engine lacked that protection.

## Reviewed dependency updates

The existing, user-approved dependency edits were checked separately from the health-rule changes. [Tokio 1.54.1](https://github.com/tokio-rs/tokio/security/advisories/GHSA-5wjr-9356-4wc8) fixes the multi-threaded `Interval` race in 1.54.0; this is a concrete upstream memory-safety fix, not a claim that this application reproduced the exploit. [rusqlite 0.41](https://github.com/rusqlite/rusqlite/releases/tag/v0.41.0) updates bundled SQLite and some iterator/callback APIs; the repository does not use the changed column-iterator or callback interfaces. Its published manifest requires Rust 1.89, already required by the existing uuid dependency. [scraper 0.29](https://github.com/rust-scraper/scraper/releases/tag/v0.29.0) changes selector error representation; repository call sites do not name the old lifetime-bearing error type. [uuid 1.29](https://github.com/uuid-rs/uuid/releases/tag/v1.29.0), [open 5.4.5](https://github.com/Byron/open-rs/releases/tag/v5.4.5), [plist 1.10.3](https://github.com/ebarnard/rust-plist/blob/v1.10.3/CHANGELOG.md) and the psl 2.1.245 data refresh remain subject to the same compilation, parsing, snapshot and native-export tests. The ignored dependency lock is retained with the local package for reproducibility; it is not silently forced into version control.

## File publication

CLI quarantine apply uses private staging and same-directory no-clobber publication for both HTML and audit, just as native review refuses occupied destinations. Existing serializer metadata and input hash/count checks remain. Two files cannot be committed as one crash-atomic filesystem transaction; an interruption may leave an audit without its HTML. Rollback assumes another writer does not replace the newly published audit path. A failed apply is not an instruction to delete existing files and retry blindly.

## Test isolation

Ordinary CLI integration tests use child-process-only private profile/configuration roots and a synthetic Safari bookmark file; they never need the caller's real browser data. Child stdin is closed and execution has a ten-second bound. All seven command tests remain enabled; success/failure is asserted explicitly, and macOS analysis must report the one synthetic bookmark. This replaces an older harness that inherited the caller's home and could read real Profiles during `analyze`/`validate`; earlier unisolated test runs are not evidence of a fixture-only session. The commands involved perform read-only local structural analysis, not URL probing. Test isolation does not change the user's environment or any toolchain settings.

## Known limits and what the tests do not establish

- Synthetic fixtures prove these behaviors, not a measured real-world precision/recall score. There is no labeled corpus spanning authenticated, regional, anti-bot and dynamic sites yet; no percentage accuracy is claimed.
- Ordinary unauthenticated 404 can hide a private resource. Structured-content rules are textual heuristics, not a browser rendering engine; disambiguating quoted/example payloads from authoritative player state still needs a dedicated acceptance slice. Confirmation remains a reviewable candidate, not automatic permission to remove a user's data.
- Source coverage remains partial for Safari custom Profile/Favorites mapping, independent Technology Preview storage and unsupported reading-list formats. A successful network pass cannot close collection gaps.
- The metadata-preserving native review exporter is not an untrusted-HTML sanitizer. It does not render imported HTML in the App, but retained active markup may run if that HTML is later opened as a web page. Use trusted browser exports; do not interpret the cleaned copy as sanitized arbitrary HTML.
- Proxy-side DNS, remote CI, Windows/Linux runtime, full accessibility and notarization need their own evidence. No live browser collection, user-URL scan or background monitor is part of this acceptance.
