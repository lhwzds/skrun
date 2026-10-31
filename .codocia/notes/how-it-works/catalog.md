---
title: Why catalog output is safe to commit
order: 90
evidence:
  - crates/skrun/src/catalog.rs:11-15 #7ae25012621e
  - crates/skrun/src/catalog.rs:385-417 #ea74ebf78ba7
  - crates/skrun/src/catalog.rs:446-550 #3ce9d0ae55e3
  - crates/skrun/src/catalog.rs:1157-1194 #3df25892b575
  - crates/skrun/src/catalog.rs:820-868 #11143a0cd3ca
  - crates/skrun/src/catalog.rs:913-952 #9a23feebd41d
  - crates/skrun/src/catalog.rs:1272-1306 #d9abc3577134
---

`catalog` never runs skills, calls git or touches the network; it only reads the directory, so its outputs can be committed and reviewed as diffs. `build_index` sorts skills by id and scripts by path and records no timestamps or absolute paths. Per skill it keeps the frontmatter (`name`, `description`, `version`, `tags`, `requires`), the entrypoint size as `bytes`, the folder's `.py`, `.sh`, `.js`/`.mjs` and `.ts` scripts (Python ones with their top-level imported modules), and relative links with whether the target exists. An unchanged directory therefore yields an identical `skills.json`, which is how `check` can call a committed copy stale.

`reconcile_registry` treats `SKILL_REGISTRY.md` as curated text. It finds the `| Skill | When to read | Entrypoint |` table, keeps surviving rows verbatim and in order, drops rows whose entrypoint is missing, not a skill, or repeated, and appends `| name | summary | [id](entrypoint) |` for unlisted skills, where the summary is the description's first sentence, cut to 160 characters. Lines outside the table and line endings are kept. The header match ignores surrounding whitespace; with no table it fails and never writes the file.
