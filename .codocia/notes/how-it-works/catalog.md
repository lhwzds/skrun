---
title: Why catalog output is deterministic
order: 90
evidence:
  - crates/skrun/src/catalog.rs:12-16 #7ae25012621e
  - crates/skrun/src/catalog.rs:403-435 #ea74ebf78ba7
  - crates/skrun/src/catalog.rs:464-568 #4fe8f5613725
  - crates/skrun/src/catalog.rs:1209-1252 #837e85b4c6f7
  - crates/skrun/src/catalog.rs:845-893 #11143a0cd3ca
  - crates/skrun/src/catalog.rs:938-986 #114d4e0bbd56
  - crates/skrun/src/catalog.rs:1447-1482 #1b80cc81666d
  - crates/skrun/src/catalog.rs:1264-1340 #133fa0d00d96
  - crates/skrun/src/catalog.rs:1381-1434 #56d4f5b495f2
---

`catalog` never runs skills, calls git or the network; only `registry --write` and `index --out` write. `build_index` sorts skills by id and scripts by path, with no timestamps or absolute paths. Per skill: frontmatter (`name`, `description`, `version`, `tags`, `requires`), entrypoint `bytes`, scripts (`.py`, `.sh`, `.js`/`.mjs`, `.ts`; Python with top-level imports) and relative links with whether each target exists. Unchanged input yields an identical `skills.json`, letting `check` flag a stale copy. It is data and never redacted: frontmatter is copied verbatim and paths kept, so check both before committing.

`reconcile_registry` treats `SKILL_REGISTRY.md` as curated text. It finds the `| Skill | When to read | Entrypoint |` table outside fenced code blocks (backtick or `~~~` fences), so fenced examples are never rewritten. It keeps valid rows verbatim and in order, drops rows with a missing, non-skill or repeated entrypoint, and appends `| name | summary | [id](entrypoint) |` for unlisted skills, summarised as the description's first sentence (max 160 characters). Cells honour `\|` and doubled-backslash escapes, which generated cells also write, so rows round-trip; an aligned separator (`:---:`) is kept. Other lines and endings stay; with no table it fails without writing.
