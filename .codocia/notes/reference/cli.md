---
title: CLI commands and flags
order: 10
evidence:
  - crates/skrun/src/main.rs:46-51 #3b092829bed3
  - crates/skrun/src/main.rs:272-301 #78201bd6e2e4
  - crates/skrun/src/main.rs:303-368 #9de4da70b642
  - crates/skrun/src/main.rs:370-431 #ea041e611b2e
  - crates/skrun/src/main.rs:544-565 #5aefde08178c
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

`--root` defaults to the skills root (see below). Text output of `list` and `show` is tab-separated `id kind version name`. Passing both a directory and `--id` to `run` is an error. `--cargo` and `--uv` exist but are not in the usage text. Errors print `Error: ...` to stderr and exit 1.
