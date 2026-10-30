---
title: Getting Started
---

# Getting Started

<sub>**English** · [中文](zh/02-getting-started.md)</sub>

You need a Rust toolchain to build `rust_binary` skills, [uv](https://docs.astral.sh/uv/) to build and run `python_uv` skills, and Python 3.11 or newer for the Python package. By default skrun invokes the plain commands `cargo` and `uv`, so both must be on your PATH.

<sub>Sources: [runtime.rs:313-337](../crates/skrun/src/runtime.rs#L313-L337) · [pyproject.toml:6-8](../pyproject.toml#L6-L8)</sub>

## Install

```bash
pip install skrun                 # Python SDK with the native extension
cargo install skrun               # the `skrun` CLI from crates.io
cargo install --path crates/skrun # or: the CLI from a checkout
```

Both packages are published: PyPI has `skrun` 0.1.0 and crates.io has `skrun` 0.1.1 (why the versions differ: [Contributing](05-contributing.md)). The PyPI job runs on `ubuntu-22.04` with Python 3.13, so the published binary wheel targets that platform; elsewhere pip falls back to the sdist and needs Rust plus maturin to compile `skrun-native`. To work from a checkout instead, see [Contributing](05-contributing.md).

<sub>Sources: [Cargo.toml:1-15](../crates/skrun/Cargo.toml#L1-L15) · [publish-python.yml:22](../.github/workflows/publish-python.yml#L22) · [publish-python.yml:34-37](../.github/workflows/publish-python.yml#L34-L37)</sub>

## Create, build and run a skill

```bash
skrun skill new --kind rust_binary --id rust-echo /tmp/rust-echo
skrun skill build /tmp/rust-echo
skrun skill run --input '{"ok":true}' /tmp/rust-echo
# {
#   "ok": true
# }
```

`new` writes a standalone Cargo project (`Cargo.toml` with an empty `[workspace]`, `src/main.rs` that echoes stdin), permissive `schema/input.json` and `schema/output.json`, a `SKILL.md`, and `artifact.json` with `entry: bin/release/rust-echo`. `build` runs Cargo and copies the binary to that `entry` path. `run` pipes the input to the binary and pretty-prints the JSON object it returns.

For Python, use `--kind python_uv`: the scaffold is a `pyproject.toml` plus an echoing `skill.py`, and `build` runs `uv lock`/`uv sync`.

<sub>Sources: [runtime.rs:623-659](../crates/skrun/src/runtime.rs#L623-L659) · [runtime.rs:814-852](../crates/skrun/src/runtime.rs#L814-L852) · [runtime.rs:932-970](../crates/skrun/src/runtime.rs#L932-L970) · [main.rs:73-81](../crates/skrun/src/main.rs#L73-L81)</sub>

## Install it and call it from Python

`install-local` copies a skill directory into the skills root (`~/.skrun/skills` by default; see [Reference](03-reference.md)):

```bash
skrun skill install-local /tmp/rust-echo
skrun skill list
skrun skill run --id rust-echo --input '{"ok":true}'
```

```python
import skrun

skrun.skill("rust-echo").call({"ok": True})        # by id, from the skills root
skrun.skill("/tmp/rust-echo").call({"ok": True})   # or by path
```

`skrun.skill` treats its argument as a path when that directory contains `artifact.json` or `SKILL.md`, otherwise as an id under the skills root. Because install copies, re-install with `--overwrite` after rebuilding a skill ([How It Works](04-how-it-works.md)).

<sub>Sources: [main.rs:82-107](../crates/skrun/src/main.rs#L82-L107) · [skill.py:236-240](../python/skrun/skill.py#L236-L240)</sub>

## Try a real example

`examples/skills` holds `rust-echo`, `python-echo`, `regex-finder` (Rust `regex` crate) and `cdp-browser` (Chrome DevTools Protocol control). The two larger ones show how an existing command-line tool becomes a skill by adding `artifact.json` and JSON schemas, and each ships a `smoke.json` input:

```bash
skrun skill build examples/skills/regex-finder
skrun skill run --input "$(cat examples/skills/regex-finder/smoke.json)" examples/skills/regex-finder
```

`regex-finder` expects `{"action": "match", "input": {"pattern": ..., "text": ...}}` (or `"action": "describe"`) and answers with an `ok`/`action`/`data`/`error` envelope. Building writes `bin/release/regex-finder` inside the example directory.

<sub>Sources: [artifact.json](../examples/skills/regex-finder/artifact.json) · [smoke.json](../examples/skills/regex-finder/smoke.json) · [main.rs:91-117](../examples/skills/regex-finder/src/main.rs#L91-L117) · [release-skills.yml:75-82](../.github/workflows/release-skills.yml#L75-L82)</sub>

## Call a skill from an agent framework

skrun has no framework adapters; a tool body is one line, and the framework's own decorator or wrapper supplies the tool schema:

```python
import skrun

def regex_finder_tool(pattern: str, text: str) -> dict:
    return skrun.skill("regex-finder").call(
        {"action": "match", "input": {"pattern": pattern, "text": text}}
    )
```

`examples/frameworks` has the same shape for LangChain, LangGraph, PydanticAI and OpenAI Agents. The LangChain example passes the tool's `arguments` dict through unchanged. The other three build `{"pattern": ..., "path": ...}`, which `regex-finder` rejects because it requires `text`; use the input shape above. `schema/input.json` in each skill is the natural source for the framework's parameter schema. Errors surface as `SkillRuntimeError`, which most frameworks report back to the model.

<sub>Sources: [langchain_tool.py:15-18](../examples/frameworks/langchain_tool.py#L15-L18) · [langgraph_node.py:10-19](../examples/frameworks/langgraph_node.py#L10-L19) · [main.rs:91-117](../examples/skills/regex-finder/src/main.rs#L91-L117) · [input.json](../examples/skills/regex-finder/schema/input.json) · [skill.py:172-173](../python/skrun/skill.py#L172-L173) · +2</sub>
