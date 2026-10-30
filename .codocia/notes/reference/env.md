---
title: Environment and default paths
order: 20
evidence:
  - crates/skrun/src/runtime.rs:50-53 #9c6610f215bc
  - crates/skrun/src/runtime.rs:719-725 #071569a1eafa
  - crates/skrun/src/runtime.rs:954-959 #6c342c39a43a
  - python/skrun/skill.py:242-248 #a469e273d384
  - crates/skrun/src/main.rs:300-308 #7ac1b9a4dc22
  - crates/skrun/src/main.rs:329 #206102ded37d
  - crates/skrun/src/main.rs:351-353 #f2e650e63d7f
  - crates/skrun/src/main.rs:369-371 #eee19c4338c0
---

| name | effect |
|---|---|
| `SKRUN_SKILLS_DIR` | skills root used by `run --id`, `install-local`, `list`, `show` and the Python SDK |
| `HOME` | fallback root `$HOME/.skrun/skills`; the CLI and Rust callers fail with `HOME is not set` if neither is set |

The CLI computes this default eagerly (`root.unwrap_or(default_skills_dir()?)`), so with neither variable set, `run --id`, `install-local`, `list` and `show` fail with `HOME is not set` even when `--root` is given.

`build` of a `rust_binary` uses `<system temp dir>/skrun-skill-targets/<id>` as Cargo's target directory unless `--target-dir` is given, so by default skill builds write no Cargo target directory into the skill directory. The Python SDK resolves the same root itself (expanding `~`), and passes its `root=` argument through when you set one.
