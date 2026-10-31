---
title: Version hashes and file modes
order: 15
evidence:
  - crates/skrun/src/store.rs:82-107 #787ca4b198b7
  - crates/skrun/src/store.rs:541-572 #aa500241dad2
  - crates/skrun/src/store.rs:828-837 #f2a33c2ac36a
  - crates/skrun/src/store.rs:1264-1328 #82be4acd5eaf
  - crates/skrun/src/store.rs:1856-1886 #cbbaab4b1dc6
---

`content_hash` feeds sha256, per file in path order, the path length as a big-endian u64, the path, the content length, then the content, so different file sets never share a hash. Its first 16 hex characters name the version directory; if that directory holds other content, `version_name` uses 32, then 64, so different content is never taken as already stored.

A version is copied beside its place and renamed in, never half written. Store directories are `0700`, stored files `0400` and their directories `0500`, records `0600`.
