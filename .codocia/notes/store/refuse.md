---
title: What add refuses
order: 20
evidence:
  - crates/skrun/src/store.rs:59-87 #9e6b86e1f303
  - crates/skrun/src/store.rs:827-849 #c2b4a1dba0a0
  - crates/skrun/src/store.rs:870-955 #bb94cc690286
  - crates/skrun/src/store.rs:957-1090 #222bca8b5277
  - crates/skrun/src/store.rs:1566-1584 #8510b9524e74
---

The source is staged in a fresh temporary directory before anything else, and a refused source never reaches the store. `add` fails on:

- a source that is itself a symlink. Symlinks inside a directory are never followed or copied; each becomes a `symlink` finding instead;
- a source, file or zip entry name that is not UTF-8;
- a zip entry with an absolute path or a path that leaves the archive root, a symlink entry, more than 5,000 entries, more than 50 MB uncompressed, or an entry compressed more than 100 times;
- no `SKILL.md` at the top level or inside a single top-level folder (so a zip of a repository folder works);
- an id that fails the id rules, or another `add` holding the same id's lock for 10 seconds.

A directory copy skips `.git`, `.venv`, `__pycache__`, `node_modules` and `target` folders, so they are neither stored nor hashed.
