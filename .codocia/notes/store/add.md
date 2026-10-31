---
title: What add stores
order: 10
evidence:
  - crates/skrun/src/store.rs:300-321 #d2090e269564
  - crates/skrun/src/store.rs:323-487 #9a952611f9fd
  - crates/skrun/src/store.rs:489-520 #aa500241dad2
  - crates/skrun/src/store.rs:746-825 #4ea140db0a67
  - crates/skrun/src/store.rs:1092-1114 #452c96bf6eb6
  - crates/skrun/src/store.rs:1651-1708 #8be5ddb950d2
  - crates/skrun/src/store.rs:1471-1500 #1e87d0ac283d
  - crates/skrun/src/catalog.rs:1205-1230 #b007c066dd9c
---

```mermaid
flowchart LR
  src["dir or zip"] --> stage["stage in a temp dir"] --> id["pick id, take locks/id.lock"] --> hash["sha256 content hash"] --> scan["scan: findings, tier"]
  scan --> copy["skills/id/hash/ (read-only)"]
  scan --> rec["records/id.json"]
```

The store root is `--store`, else `$SKRUN_STORE`, else `~/.skrun/store`. The id is `--id`, else the source's own name (the directory name or the zip file stem, taken from the real path, so `add .` works), else the frontmatter `name` with YAML quotes stripped when the source name is not a valid id ([[reference]]). Preferring the source name keeps two authors' skills with the same `name` apart.

The content hash covers every file's path and bytes; a short prefix names the version directory, which is built beside its place and renamed in, so a version never appears half written. `records/<id>.json` lists every version: hash, `--from`, time, file and byte counts, frontmatter name and description, scripts with language and Python imports, findings, tier and reasons. It is written to a temporary file and renamed, under the per-id lock. Adding identical content stores nothing new; only a scan that now differs refreshes that version's scripts, findings and tier.
