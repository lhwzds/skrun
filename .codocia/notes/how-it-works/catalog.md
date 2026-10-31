---
title: Why catalog output is deterministic
order: 90
evidence:
  - crates/skrun/src/catalog.rs:14-18 #7ae25012621e
  - crates/skrun/src/catalog.rs:423-455 #ea74ebf78ba7
  - crates/skrun/src/catalog.rs:484-588 #4fe8f5613725
  - crates/skrun/src/catalog.rs:3592-3635 #837e85b4c6f7
  - crates/skrun/src/catalog.rs:1335-1379 #ea70c2e9f253
  - crates/skrun/src/catalog.rs:2446-2503 #2345a196e61e
  - crates/skrun/src/catalog.rs:4009-4044 #1b80cc81666d
  - crates/skrun/src/catalog.rs:3647-3840 #92885df1d766
  - crates/skrun/src/catalog.rs:3907-3960 #56d4f5b495f2
  - crates/skrun/src/catalog.rs:3962-4003 #a8acb1b5fcc3
  - crates/skrun/src/catalog.rs:2396-2444 #d523bb53cf92
---

`catalog` never runs skills, git or network calls; only `registry --write` and `index --out` write. `build_index` sorts skills by id and scripts by path, without timestamps or absolute paths. Per skill: frontmatter, entrypoint `bytes`, scripts (`.py`, `.sh`, `.js`/`.mjs`, `.ts`; Python imports) and relative links (parsed like registry cells) with whether each exists. Same input, same `skills.json`, so `check` flags a stale copy. Nothing is redacted: review it before committing.

`reconcile_registry` edits only the `| Skill | When to read | Entrypoint |` table of the curated `SKILL_REGISTRY.md`, outside fenced code (backtick or `~~~`, closed only in the opening container: top level, list item or quote; a backticked info string means inline code). Valid rows stay verbatim and ordered; rows with a missing, non-skill or repeated entrypoint go; unlisted skills get `| name | summary | [id](entrypoint) |` (description's first sentence, ≤160 characters). Cells honour `\|` and `\\`; paths with parentheses, spaces, `<` or `>` use `[id](<path>)`. Separators, other lines and endings stay; with no table it fails unwritten. Known gaps: ordered-list fences closed at 4+ spaces stay open (`registry` then finds no table), reversed list/quote nesting may close one early, `>` paths don't round-trip.
