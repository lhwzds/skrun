---
title: Contributing
---

# Contributing

<sub>**English** · [中文](zh/05-contributing.md)</sub>

The Cargo workspace has two members: `crates/skrun` (default member: runtime library and CLI) and `crates/py` (`skrun-native`). PyO3 is behind the optional `python-module` feature, so `cargo build` and `cargo test` at the root never need a Python interpreter; only maturin and the native checks enable it. The module map is in [Code structure](structure.md).

<sub>Sources: [Cargo.toml:1-17](../Cargo.toml#L1-L17) · [Cargo.toml:13-15](../crates/py/Cargo.toml#L13-L15) · [build.rs](../crates/py/build.rs)</sub>

## Local development loop

```bash
make test     # cargo test + python unittest of python/tests
make lint     # cargo fmt --check + clippy -D warnings
make python   # scripts/check.sh ci: full package loop
make check    # lint + test + python
```

`scripts/check.sh ci` creates a venv, installs maturin, runs the Python contract tests, tests and lints `skrun-native` with `--features python-module`, then `develop`, `smoke`, `build` and `sdist` through `python/scripts/package.py`. Each step is also a subcommand (`bootstrap`, `python`, `rust`, `develop`, `smoke`, `build`, `sdist`). It reads `PYTHON_BIN` (`python3`), `PYTHON_VENV` (`/tmp/skrun-python-venv`), `CARGO_TARGET_DIR` (`/tmp/skrun-python-target`) and `WHEEL_DIR` (`/tmp/skrun-python-dist`). `package.py` sets `PYO3_PYTHON` and `CARGO_TARGET_DIR` itself; the default target lives in `/tmp` so native artifacts are not built or executed from an external macOS volume. `make clean` removes those three `/tmp` paths.

<sub>Sources: [Makefile](../Makefile) · [check.sh:7-13](../scripts/check.sh#L7-L13) · [check.sh:59-89](../scripts/check.sh#L59-L89) · [package.py:13-41](../python/scripts/package.py#L13-L41)</sub>

## What the smoke test proves

`package.py smoke` runs from `/tmp`, so it imports the installed package rather than the source tree. It writes a throwaway `rust_binary` skill whose entry is a `#!/bin/sh` script running `cat`, then asserts that `skrun.skill(root).call({'native': True})` returns the same dict. A shell script is enough because `rust_binary` only requires an executable file at `entry`, which also makes this a quick way to wrap any existing executable for testing.

<sub>Sources: [package.py:64-88](../python/scripts/package.py#L64-L88)</sub>

## CI and releases

| workflow | trigger | does |
|---|---|---|
| `python.yml` | push / PR to `main` touching code | the `check.sh` steps, uploads the wheel |
| `publish-crates.yml` | tag `v*` or manual | `cargo test -p skrun`, dry-run, `cargo publish -p skrun` |
| `publish-python.yml` | tag `v*` or manual | `scripts/check.sh ci`, then PyPI publish (`skip-existing`) |
| `release-skills.yml` | tag `<skill>@<version>` or manual | builds, smoke-runs and attaches `<skill>@<version>-x86_64-linux.tar.gz` (`artifact.json`, `SKILL.md`, `schema`, `bin`) to a GitHub release |

`release-skills.yml` accepts only `regex-finder` and `cdp-browser`, and uses the CLI itself to build and run each example's `smoke.json`. Crates.io and PyPI publishing use trusted publishing (`id-token: write`), not stored tokens. Versions are set separately: the workspace `Cargo.toml` says 0.1.1 (the published crate), while `pyproject.toml` and `python/skrun/__init__.py` say 0.1.0 (the published PyPI package). Bump all three when releasing; `skip-existing` means an unbumped Python version is silently not re-uploaded.

<sub>Sources: [python.yml:1-20](../.github/workflows/python.yml#L1-L20) · [publish-crates.yml:1-52](../.github/workflows/publish-crates.yml#L1-L52) · [publish-python.yml:1-11](../.github/workflows/publish-python.yml#L1-L11) · [publish-python.yml:50-57](../.github/workflows/publish-python.yml#L50-L57) · [release-skills.yml:1-103](../.github/workflows/release-skills.yml#L1-L103) · +3</sub>

## Adding a skill kind or command

A new kind touches every `match` on `ArtifactKind` in `runtime.rs`: `validate`, `scaffold_skill`, `build_skill`, `run_skill` and `kind_label`, plus `parse_kind` and the duplicate `kind_label` in `main.rs`. The Python side treats `kind` as a plain string and needs no change unless the kind needs new options. Keep the runtime contract intact: anything that executes must still take one JSON value on stdin and return one JSON object on stdout.

The runtime, CLI, binding and SDK modules open with a doc header (`Owns`, `Must Not`, `Inputs`, `Outputs`, `Depends On`, `Verify`). Update it when you move responsibility, and respect the `Must Not` lists: the runtime does not plan, parse `@skill` mentions, render UI or hold durable state. New operations belong in `runtime.rs` first and are then exposed through `main.rs` and `crates/py/src/lib.rs`, never implemented in Python.

<sub>Sources: [runtime.rs:55-61](../crates/skrun/src/runtime.rs#L55-L61) · [runtime.rs:586-592](../crates/skrun/src/runtime.rs#L586-L592) · [main.rs:544-573](../crates/skrun/src/main.rs#L544-L573) · [runtime.rs:3-17](../crates/skrun/src/runtime.rs#L3-L17)</sub>
