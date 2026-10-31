---
title: What add refuses
order: 20
evidence:
  - crates/skrun/src/store.rs:73-116 #e8a9784b71d1
  - crates/skrun/src/store.rs:897-919 #c2b4a1dba0a0
  - crates/skrun/src/store.rs:940-1025 #bb94cc690286
  - crates/skrun/src/store.rs:1027-1262 #ae15ade094c5
  - crates/skrun/src/store.rs:1952-1970 #8510b9524e74
---

The source is staged in a fresh temporary directory first; a refused source never reaches the store. `add` fails on:

- a source that is itself a symlink. Symlinks inside a directory are never followed or copied; each becomes a `symlink` finding;
- a source, file or zip entry name that is not UTF-8;
- a zip entry with an absolute path or one leaving the archive root, a symlink entry, more than 5,000 entries, more than 50 MB uncompressed, or an entry compressed over 100 times, counted on the bytes actually written, not just declared sizes;
- no `SKILL.md` at the top level or inside a single top-level folder (so a zip of a repository folder works);
- an invalid id, or another `add` holding the id's lock for 10 seconds.

Directory copies skip `.git`, `.venv`, `__pycache__`, `node_modules` and `target`.
