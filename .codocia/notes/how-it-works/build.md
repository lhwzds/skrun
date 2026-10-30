---
title: Builds
order: 40
evidence:
  - crates/skrun/src/runtime.rs:661-673 #4f458269187f
  - crates/skrun/src/runtime.rs:946-992 #6fede6f7ebaa
  - crates/skrun/src/runtime.rs:1068-1080 #fdcfc48d9a53
---

`rust_binary`: run `cargo build --manifest-path <dir>/Cargo.toml --target-dir <target>` with `--release` when the profile is `release`, otherwise `--profile <name>`, then copy `<target>/<dir>/<id>` (plus the platform exe suffix) to `entry`, where `<dir>` is Cargo's output directory: `debug` for `dev` and `test`, `release` for `release` and `bench`, otherwise the profile name. The binary name must therefore equal the skill id. The copy goes to `entry` whatever the profile, so a debug build still lands in `bin/release/...` with the scaffolded entry.

`python_uv`: run `uv lock --project <dir>` only when `uv.lock` is missing, then `uv sync --project <dir> --locked`. The lock file is part of the skill, so installs are reproducible.

Both build steps capture output and report only stderr on failure. `markdown` skills cannot be built.
