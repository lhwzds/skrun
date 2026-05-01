# skrun

`skrun` is the Python SDK for the skrun executable skill runtime. The
distribution name and import name are both `skrun`.

skrun is not a replacement for LangChain, LangGraph, PydanticAI, OpenAI
Agents, or other main agent frameworks. It provides a local runtime for
building, installing, and calling executable skills from those frameworks.

## Install

```bash
pip install skrun
```

## Use An Installed Skill

```python
import skrun

result = skrun.skill("regex-finder").call(
    {
        "pattern": "TODO",
        "path": ".",
    }
)
```

Skill IDs resolve under `~/.skrun/skills` by default. Set
`SKRUN_SKILLS_DIR` to use a different local skill directory.

## Artifact Kinds

The first executable skill runtime supports:

- `rust_binary`: Cargo-built executable skills called with stdin/stdout JSON.
- `python_uv`: uv-managed Python skills called with stdin/stdout JSON.

## Agent Framework Integration

Wrap `skrun.skill(...).call(...)` in the tool abstraction of your existing
agent framework. The skrun repository includes dependency-light examples for
LangChain, LangGraph, OpenAI Agents-style function tools, and PydanticAI-style
tool bodies.
