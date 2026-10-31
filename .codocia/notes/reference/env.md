---
title: Environment and default paths
order: 20
evidence:
  - crates/skrun/src/runtime.rs:50-53 #9c6610f215bc
  - crates/skrun/src/runtime.rs:719-725 #071569a1eafa
  - crates/skrun/src/runtime.rs:954-959 #6c342c39a43a
  - python/skrun/skill.py:242-248 #a469e273d384
  - crates/skrun/src/main.rs:455-463 #9bcc35be9523
  - crates/skrun/src/main.rs:488 #206102ded37d
  - crates/skrun/src/main.rs:512-514 #f2e650e63d7f
  - crates/skrun/src/main.rs:532-534 #2d39a2916a38
  - crates/skrun/src/store.rs:300-316 #8a1a312a927a
---

| name | effect |
|---|---|
| `SKRUN_SKILLS_DIR` | skills root used by `run --id`, `install-local`, `list`, `show` and the Python SDK |
| `HOME` | fallback root `$HOME/.skrun/skills`; the CLI and Rust callers fail with `HOME is not set` if neither is set |
| `SKRUN_STORE` | store of `catalog add` and `inspect` when `--store` is absent, else `$HOME/.skrun/store` ([[store]]) |

The CLI computes this default eagerly (`root.unwrap_or(default_skills_dir()?)`), so with neither variable set, `run --id`, `install-local`, `list` and `show` fail with `HOME is not set` even when `--root` is given.

`build` of a `rust_binary` uses `<system temp dir>/skrun-skill-targets/<id>` as Cargo's target directory unless `--target-dir` is given, so skill builds leave no Cargo target directory in the skill directory. The Python SDK resolves the same root itself (expanding `~`), and passes its `root=` argument through when you set one.
