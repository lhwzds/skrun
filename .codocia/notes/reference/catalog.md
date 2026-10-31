---
title: Catalog commands
order: 15
evidence:
  - crates/skrun/src/main.rs:130-152 #e9bf6889ef29
  - crates/skrun/src/main.rs:508-556 #078cae122961
  - crates/skrun/src/main.rs:435-506 #67150dea7076
  - crates/skrun/src/catalog.rs:60-66 #87ffa01f4f85
  - crates/skrun/src/catalog.rs:353-400 #22fd5b8409ad
  - crates/skrun/src/catalog.rs:571-694 #64df15467c4d
  - crates/skrun/src/catalog.rs:193-208 #2f9a08ba011c
  - crates/skrun/src/catalog.rs:240-276 #931a320d30be
  - crates/skrun/src/catalog.rs:1062-1077 #b4503feb606c
---

`skrun catalog` works on a directory of guidance skills: non-hidden subdirectories with a `SKILL.md`, plus top-level `*.md` except `SKILL_REGISTRY.md`, `README.md`, `README_CN.md`, `AGENTS.md` and `CLAUDE.md`.

| command | flags (default) | exits 1 when |
|---|---|---|
| `index <dir>` | `--out <file>` (stdout) | the directory cannot be read |
| `registry <dir>` | `--registry` (`<dir>/SKILL_REGISTRY.md`), `--write` | without `--write`, the table drifted; always, the file is missing, has no table, or cannot be read or written |
| `check <dir>` | `--registry`, `--index` | any error |

`check` errors on: a folder skill missing frontmatter `name` or `description`, a broken relative link, a registry without the table or drifted, a `skills.json` differing from a fresh index, and a line that looks like a private key or an AWS, OpenAI, GitHub or Slack token. Its one warning: a frontmatter `name` differing from the directory name. Registry and index are checked when present at the default path or passed.

`check` and `registry` print user text (names, link targets, paths, malformed rows, unknown options) matching those patterns as `<redacted>` (`redact`); a secret finding gives only `path:line`, the path masked too. `skills.json` keeps real paths ([[how-it-works]]).
