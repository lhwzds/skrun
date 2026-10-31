---
title: Catalog commands
order: 15
evidence:
  - crates/skrun/src/main.rs:125-152 #5548e966f3e4
  - crates/skrun/src/main.rs:506-523 #91b6fb243943
  - crates/skrun/src/main.rs:433-504 #d4442ae00df9
  - crates/skrun/src/catalog.rs:57-63 #87ffa01f4f85
  - crates/skrun/src/catalog.rs:336-382 #1895ec962f45
  - crates/skrun/src/catalog.rs:553-670 #91f41c946afb
  - crates/skrun/src/catalog.rs:187-202 #2f9a08ba011c
---

`skrun catalog` works on a directory of guidance skills, not on one artifact: each non-hidden subdirectory with a `SKILL.md`, plus each top-level `*.md` except `SKILL_REGISTRY.md`, `README.md`, `README_CN.md`, `AGENTS.md` and `CLAUDE.md`.

| command | flags (default) | exits 1 when |
|---|---|---|
| `index <dir>` | `--out <file>` (stdout) | the directory cannot be read |
| `registry <dir>` | `--registry` (`<dir>/SKILL_REGISTRY.md`), `--write` | without `--write`, the table drifted; in both modes, the file is missing, has no table, or cannot be read or written (`--write` exits 0 after repairing) |
| `check <dir>` | `--registry`, `--index` | any error |

`check` reports as errors: a folder skill without frontmatter `name` or `description`, a broken relative link, a registry without the table or with drift, a `skills.json` that differs from a fresh index, and a line that looks like a private key or an AWS, OpenAI, GitHub or Slack token (printed as `path:line`, never the value). The only warning is a frontmatter `name` that differs from the directory name. The registry is checked only if it exists or `--registry` is given; the index only if `<dir>/skills.json` exists or `--index` is given. [[how-it-works]] explains why the outputs are deterministic.
