---
title: CLI commands and flags
order: 10
evidence:
  - crates/skrun/src/main.rs:47-52 #3b092829bed3
  - crates/skrun/src/main.rs:272-301 #d0145b9e1c53
  - crates/skrun/src/main.rs:303-368 #6ca7926c42be
  - crates/skrun/src/main.rs:370-433 #8d853a4fca78
  - crates/skrun/src/main.rs:577-598 #0555d91c5b26
  - crates/skrun/src/runtime.rs:313-337 #e781ef9878e1
---

Skill commands live under `skrun skill`; `skrun catalog` is below. Positional `<dir>` / `<id>` may appear anywhere among the flags.

| command | flags (default) |
|---|---|
| `new <dir>` | `--kind markdown\|guidance\|rust_binary\|python_uv` (required), `--id` (required), `--name` (title-cased id: `regex-finder` → `Regex Finder`), `--version` (`0.1.0`) |
| `build <dir>` | `--profile` (`release`), `--target-dir`, `--cargo` (`cargo`), `--uv` (`uv`) |
| `run <dir>` or `run --id <id>` | `--input` (`{}`; must be a JSON object), `--timeout` seconds (`60`), `--uv` (`uv`), `--root` (with `--id` only) |
| `install-local <dir>` | `--root`, `--id` (rename on install), `--overwrite` |
| `list` | `--root`, `--format text\|json` (`text`) |
| `show <id>` | `--root`, `--format text\|json` (`text`) |

`--root` defaults to the skills root (see below). Text output of `list` and `show` is tab-separated `id kind version name`. Passing both a directory and `--id` to `run` is an error. `--cargo` and `--uv` exist but are not in the usage text. Errors print `Error: ...` to stderr and exit 1; echoed arguments and paths that look like a secret print as `<redacted>`.
