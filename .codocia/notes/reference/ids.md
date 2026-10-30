---
title: Id and path rules
order: 50
evidence:
  - crates/skrun/src/runtime.rs:789-826 #eb7a215877ac
  - crates/skrun/src/runtime.rs:206-241 #057886d48d0b
---

An id must be non-empty, contain only ASCII letters, digits, `-` and `_`, and start with a letter or digit. The same check applies to `--id` on `install-local`, so an installed directory name is always a safe single path segment.

For the executable kinds (`rust_binary`, `python_uv`), `entry`, `schema.input` and `schema.output` must be non-empty relative paths with no `..` or root components: the check is lexical, so a symlink inside the directory can still resolve outside it. A `markdown` artifact's `schema.*` paths are not checked.
