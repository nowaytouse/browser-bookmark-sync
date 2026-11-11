# Project guidance

## Private code index and QA

A private index may be available at `${XDG_STATE_HOME:-~/.local/state}/project-indexes/<checkout-basename>/latest.json`. Verify its full Git HEAD and timestamp before use; rebuild from current committed source when stale. Keep index output private and uncommitted. Use synthetic fixtures for privacy and QA; exclude vaults, logs, and real dumps.
