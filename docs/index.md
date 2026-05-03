---
title: skrun
description: Build and run executable skills for AI agents.
covers:
  - crates/*/src/*.rs
  - crates/py/build.rs
  - python/skrun/*.py
---

# skrun

skrun builds and runs executable skills for AI agent frameworks.
It packages Rust binaries and uv-backed Python projects behind one local
stdin/stdout JSON contract, then lets an agent call those skills without giving
up ownership of planning, chat, memory, or graph state.

## Why skrun

- Build Rust binary skills as portable local artifacts.
- Build uv-backed Python skills with the same executable contract.
- Install skills under a local root and call them by id or path.
- Keep skill execution separate from the agent framework's main loop.
- Call skills through CLI, Rust runtime, or the Python `skrun` package.
- Use documentation coverage checks to keep humans and coding agents aligned.

## Module Map

The important boundary is the executable skill runtime. The Rust workspace
contains crates for skill metadata, artifact build/run, protocol types, command
execution, and Python bindings. The Python package mirrors the Rust runtime
boundary through PyO3 instead of shipping a separate subprocess fallback.

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

## What skrun Does Not Own

skrun is not a replacement for a personal coding agent, TUI, graph engine, or
chat runtime. Agent frameworks such as RestFlow, Codex-style tools, LangGraph,
or custom agents keep owning the model loop. skrun owns the executable skill
artifact and the local call boundary.

## Documentation Map

- [Quickstart](./guides/quickstart.md) covers installation and the first local skill call.
- [Examples](./guides/examples.md) covers the bundled binary skill examples.
- [Frameworks](./guides/frameworks.md) shows how to wrap skills from existing agent frameworks.
