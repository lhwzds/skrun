---
title: Why catalog output is deterministic
order: 90
evidence:
  - crates/skrun/src/catalog.rs:12-16 #7ae25012621e
  - crates/skrun/src/catalog.rs:403-435 #ea74ebf78ba7
  - crates/skrun/src/catalog.rs:464-568 #4fe8f5613725
  - crates/skrun/src/catalog.rs:1299-1342 #837e85b4c6f7
  - crates/skrun/src/catalog.rs:845-893 #11143a0cd3ca
  - crates/skrun/src/catalog.rs:938-1004 #f0d518782d14
  - crates/skrun/src/catalog.rs:1621-1656 #1b80cc81666d
  - crates/skrun/src/catalog.rs:1354-1488 #f4e12a83e3dc
  - crates/skrun/src/catalog.rs:1528-1581 #56d4f5b495f2
  - crates/skrun/src/catalog.rs:1583-1616 #32eac457ff48
---

`catalog` never runs skills, calls git or the network; only `registry --write` and `index --out` write. `build_index` sorts skills by id and scripts by path, with no timestamps or absolute paths. Per skill: frontmatter (`name`, `description`, `version`, `tags`, `requires`), entrypoint `bytes`, scripts (`.py`, `.sh`, `.js`/`.mjs`, `.ts`; Python with top-level imports) and relative links with whether each target exists. Same input, same `skills.json`, so `check` flags a stale copy. Nothing is redacted: review frontmatter and paths before committing.

`reconcile_registry` treats `SKILL_REGISTRY.md` as curated text and edits only the `| Skill | When to read | Entrypoint |` table outside fenced code (backtick or `~~~`, also in lists and quotes; a backtick in the info string means inline code). Valid rows stay verbatim, in order; rows with a missing, non-skill or repeated entrypoint go; unlisted skills get `| name | summary | [id](entrypoint) |`, summarised by the description's first sentence (≤160 characters). Cells honour `\|` and `\\` escapes; a path with parentheses, spaces, `<` or `>` is written `[id](<path>)`; both forms parse, so rows round-trip (a path with `>` not yet). Aligned separators, other lines and endings stay; with no table it fails without writing.
