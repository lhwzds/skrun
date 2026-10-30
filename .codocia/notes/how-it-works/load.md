---
title: Loading a skill directory
order: 10
evidence:
  - crates/skrun/src/runtime.rs:594-609 #8693b9f54dd4
  - crates/skrun/src/runtime.rs:427-447 #bdda18fa8068
  - crates/skrun/src/runtime.rs:392-425 #22094cc7e056
---

If `artifact.json` exists, it is decoded and then `apply_skill_markdown` fills only the metadata fields the manifest left empty (`description`, `tags`, `suggested_tools`, `content`, `source_ref`) from `SKILL.md`. The manifest always wins; `SKILL.md` is the human-facing description that agents can read. Without `artifact.json`, the directory must contain `SKILL.md` and becomes a `markdown` skill named after the directory. Either way `validate` runs last, so every caller — CLI, Rust library, Python — sees the same errors for a bad manifest.
