---
title: CLI commands and flags
order: 10
evidence:
  - crates/skrun/src/main.rs:49-66 #6e74435c913a
  - crates/skrun/src/main.rs:364-395 #95871a859219
  - crates/skrun/src/main.rs:397-468 #cb25a829034f
  - crates/skrun/src/main.rs:470-537 #d398a66d0d66
  - crates/skrun/src/main.rs:939-968 #4f7641165e7a
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

`--root` defaults to the skills root (below). `list` and `show` print tab-separated `id kind version name`. `run` rejects a directory plus `--id`. `--cargo` and `--uv` are missing from the usage text. Errors print `Error: ...` to stderr and exit 1, or 2 for a usage error (unknown, missing or invalid argument); echoed arguments and paths that look like a secret print as `<redacted>`.
