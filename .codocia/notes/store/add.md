---
title: What add stores
order: 10
evidence:
  - crates/skrun/src/store.rs:363-539 #493d77bcb79c
  - crates/skrun/src/store.rs:805-895 #8f9099a3a5ad
  - crates/skrun/src/store.rs:2037-2097 #dafbaa6cfa33
  - crates/skrun/src/catalog.rs:1208-1233 #b007c066dd9c
---

```mermaid
flowchart LR
  src["dir or zip"] --> stage["stage in a temp dir"] --> id["pick id, take locks/id.lock"] --> hash["sha256 content hash"] --> scan["scan: findings, tier"]
  scan --> copy["skills/id/hash/ (read-only)"]
  scan --> rec["records/id.json"]
```

The id is `--id`, else the source's name (directory or zip stem, from the real path, so `add .` works), else the unquoted frontmatter `name` if that is not a valid id ([[reference]]). Two sources with the same name become versions of one id unless `--id` separates them.

`records/<id>.json` lists each version: hash, `--from`, time, file and byte counts, frontmatter name and description, scripts with language and imports, findings, tier and reasons. It is written atomically under the per-id lock. Identical content stores nothing new; a scan that now differs refreshes that version's scripts, findings and tier.
