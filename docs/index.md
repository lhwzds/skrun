---
title: skrun
description: Executable skills for AI agents.
covers:
  - crates/*/src/*.rs
  - crates/py/build.rs
  - python/skrun/*.py
---

# skrun

skrun is a Python-first executable skill runtime for AI agent frameworks.
It lets an agent discover, build, install, and call local skills without taking
over the main agent loop.

## Why skrun

- Build skills as small local executables.
- Call Rust binary skills through a Python API.
- Run uv-backed Python skills with the same JSON protocol.
- Keep skill execution separate from the agent framework.
- Use documentation coverage checks to keep humans and coding agents aligned.

## Module Map

The Rust workspace is split into narrow crates for skills, executable runtime,
protocol types, command execution, storage traits, provider auth, events, and
Python bindings. The Python package mirrors those runtime boundaries instead of
shipping a separate fallback implementation.

## Install

```bash
pip install skrun
```

```bash
cargo install skrun
```

## Minimal Python Use

```python
import skrun

result = skrun.skill("regex-finder").call({
    "action": "match",
    "input": {
        "pattern": "\\d+",
        "text": "abc 123"
    }
})
```

## Documentation Map

- [Quickstart](./guides/quickstart.md) covers installation and the first local skill call.
- [Examples](./guides/examples.md) covers the bundled binary skill examples.
- [Frameworks](./guides/frameworks.md) shows how to wrap skills from existing agent frameworks.
