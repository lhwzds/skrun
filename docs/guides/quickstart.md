---
title: Quickstart
description: Install skrun and run the first executable skill.
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

skrun exposes the same executable skill runtime to Rust, Python, and the CLI.
The first contract is intentionally small: one JSON value goes to stdin and one
JSON object comes back from stdout.

## Python Package

```bash
pip install skrun
```

```python
import skrun

result = skrun.skill("/path/to/skill").call({"ok": True})
print(result)
```

Skill IDs resolve under `~/.skrun/skills` by default. Set `SKRUN_SKILLS_DIR` to
use another local skill root.

## Rust CLI

```bash
cargo install skrun
```

```bash
skrun skill new --kind rust_binary --id rust-echo /tmp/rust-echo
skrun skill build /tmp/rust-echo
skrun skill run --input '{"ok":true}' /tmp/rust-echo
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

The CLI, Rust runtime, PyO3 bridge, and Python package all use this same
contract. Changes to any one of those surfaces should be reviewed against this
quickstart before refreshing the Codocia snapshot.
