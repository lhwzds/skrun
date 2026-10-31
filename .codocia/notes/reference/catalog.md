---
title: Catalog commands
order: 15
evidence:
  - crates/skrun/src/main.rs:173-211 #3f22873d80e0
  - crates/skrun/src/main.rs:783-856 #0da533e9634d
  - crates/skrun/src/main.rs:539-771 #9c54b6ba6e24
  - crates/skrun/src/catalog.rs:63-69 #87ffa01f4f85
  - crates/skrun/src/catalog.rs:370-417 #22fd5b8409ad
  - crates/skrun/src/catalog.rs:588-711 #64df15467c4d
  - crates/skrun/src/catalog.rs:210-225 #2f9a08ba011c
  - crates/skrun/src/catalog.rs:257-293 #931a320d30be
  - crates/skrun/src/catalog.rs:1820-1835 #b4503feb606c
  - crates/skrun/src/catalog.rs:1837-1847 #f7ef48b3d4fa
  - crates/skrun/src/catalog.rs:864-899 #f9d1d15f23a1
---

`skrun catalog` works on a directory of guidance skills: non-hidden subdirectories with a `SKILL.md`, plus top-level `*.md` except `SKILL_REGISTRY.md`, `README.md`, `README_CN.md`, `AGENTS.md` and `CLAUDE.md`. Its `add` and `inspect` work on the store instead ([[store]]).

| command | flags (default) | exits 1 when |
|---|---|---|
| `index <dir>` | `--out <file>` (stdout) | the directory cannot be read |
| `registry <dir>` | `--registry` (`<dir>/SKILL_REGISTRY.md`), `--write` | without `--write`, the table drifted; always, the file is missing, has no table, or cannot be read or written |
| `check <dir>` | `--registry`, `--index` | any error |
| `search <dir> <query>...` | `--kind folder\|flat`, `--has-scripts`, `--limit` (`10`), `--json` | nothing matches (`--json` prints `[]`) |

`check` errors on: a folder skill missing frontmatter `name` or `description`, a broken relative link, a missing or drifted registry table, a stale `skills.json`, and a line holding a secret. Its one warning: a frontmatter `name` differing from the directory name.

Secrets are private-key blocks and tokens starting `AKIA`, `sk-` (incl. `sk-proj-`, `sk-ant-`), `gh[pousr]_`, `github_pat_`, `glpat-`, `xox[baprs]-` or `AIza`. `check`, `registry` and `search` print matching user text as `<redacted>` (`redact`); a secret finding gives only `path:line`. `skills.json` keeps real paths ([[how-it-works]]).
