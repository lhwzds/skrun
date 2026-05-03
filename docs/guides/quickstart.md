---
title: Quickstart
description: Build and run the first executable skill.
covers:
  - crates/cli/src/main.rs
  - crates/skrun/src/*.rs
  - crates/runtime/src/*.rs
  - crates/py/src/*.rs
  - crates/py/build.rs
  - python/skrun/*.py
  - python/scripts/*.py
  - python/tests/test_core_contract.py
  - python/tests/test_package_script.py
  - python/tests/test_skill_runtime.py
  - scripts/*.sh
---

# Quickstart

skrun exposes one executable skill runtime to Rust, Python, and the CLI.
The core loop is small: create an artifact, build it, run it with one JSON
input, and read one JSON object from stdout.

## Install

Use the Python package when an agent or framework wants to call installed
skills:

```bash
pip install skrun
```

Use the Rust CLI when you are creating, building, installing, or running skill
artifacts directly:

```bash
cargo install skrun
```

## Create, Build, Run

```bash
skrun skill new --kind rust_binary --id rust-echo /tmp/rust-echo
skrun skill build /tmp/rust-echo
skrun skill run --input '{"ok":true}' /tmp/rust-echo
```

That same command shape applies to uv-backed Python skills:

```bash
skrun skill new --kind python_uv --id python-echo /tmp/python-echo
skrun skill build /tmp/python-echo
skrun skill run --input '{"ok":true}' /tmp/python-echo
```

## Call From Python

```python
import skrun

result = skrun.skill("/tmp/rust-echo").call({"ok": True})
```

Skill IDs resolve under `~/.skrun/skills` by default. Set `SKRUN_SKILLS_DIR` to
use another local skill root.

## Install Then Call By Id

```bash
skrun skill install-local --root ~/.skrun/skills --overwrite /tmp/rust-echo
```

```python
import skrun

result = skrun.skill("rust-echo").call({"ok": True})
```

## Artifact Layout

Every executable skill directory contains an `artifact.json` manifest:

```json
{
  "schema_version": 1,
  "kind": "rust_binary",
  "id": "regex-finder",
  "name": "Regex Finder",
  "version": "0.1.0",
  "entry": "bin/release/regex-finder",
  "protocol": {
    "transport": "stdio-json",
    "input": "single-json-value",
    "output": "single-json-value"
  }
}
```

## Runtime Contract

- stdin receives one JSON value.
- stdout must return one JSON object.
- stderr is diagnostics.
- a non-zero exit code is a skill failure.
- streaming is intentionally outside the first contract.
- the agent framework owns planning, model calls, and user interaction.

The CLI, Rust runtime, PyO3 bridge, and Python package all use this same
contract. Changes to any one of those surfaces should be reviewed against this
quickstart before refreshing the Codocia snapshot.
