---
title: Install and list
order: 50
evidence:
  - crates/skrun/src/runtime.rs:727-787 #1e7b0803d135
  - crates/skrun/src/runtime.rs:1093-1110 #99c94bcb4f30
  - crates/skrun/src/runtime.rs:760-766 #dacd1d8f69e5
---

`install_local_skill` validates the source, then recursively copies it to `<root>/<id>`. An existing target fails with `installed skill already exists` unless `overwrite` is set; with `overwrite`, the target is deleted first, and installing a directory onto itself returns early without copying. The copy is re-loaded and returned; only its metadata is checked, so an unbuilt Rust skill installs fine and fails when run. Built binaries are copied, but directories named `.venv`, `__pycache__`, `target` or `.git` are skipped at any depth unless the artifact's `entry` lies inside one; a copied `.venv` would still point at the old interpreter, and uv rebuilds it on the next run. Renaming with `--id` changes only the directory name; the copied `artifact.json` keeps its original `id`.

`list_installed_skills` scans the root's direct subdirectories that hold `artifact.json` or `SKILL.md`, silently skips those that fail to load, and sorts by id. A missing root lists as empty. Use `skill show <id>` to see the load error for a skill that is missing from the list.
