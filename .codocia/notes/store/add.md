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

The id is `--id`, else the source's own name (directory name or zip stem, from the real path, so `add .` works), else the frontmatter `name` without YAML quotes when the source name is not a valid id ([[reference]]). So two authors' same-named skills stay apart.

`records/<id>.json` lists each version: hash, `--from`, time, file and byte counts, frontmatter name and description, scripts with language and imports, findings, tier and reasons. It is written to a temporary file and renamed under the per-id lock. Identical content stores nothing new; a scan that now differs refreshes that version's scripts, findings and tier.
