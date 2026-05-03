---
title: Framework Integration
description: Call executable skills from existing agent frameworks.
covers:
  - examples/frameworks/*.py
  - python/tests/test_framework_examples.py
---

# Framework Integration

skrun does not replace RestFlow, LangChain, LangGraph, PydanticAI, OpenAI
Agents, or any other main agent framework. It gives those frameworks a stable
way to call local executable skills while they keep ownership of planning,
model calls, memory, graph state, and user interaction.

## Minimal Wrapper

```python
import skrun


def regex_finder_tool(arguments: dict) -> dict:
    return skrun.skill("regex-finder").call(arguments)
```

The wrapper can be registered as a framework-specific tool function.

The framework should treat skrun like a local tool boundary: validate the tool
arguments, call the skill, and return the JSON result to the model or graph.

## LangChain Shape

```python
from langchain_core.tools import tool
import skrun


@tool
def regex_finder(pattern: str, text: str) -> dict:
    """Run regex matching through a local skrun skill."""
    return skrun.skill("regex-finder").call({
        "action": "match",
        "input": {
            "pattern": pattern,
            "text": text,
        },
    })
```

## LangGraph Shape

Use the same wrapper function as a node or as a tool bound to the model. skrun
only owns executable skill invocation; LangGraph still owns graph state and
control flow.

## Design Rule

Keep these responsibilities separate:

- the agent framework owns planning, model calls, chat state, and graph state
- skrun owns skill discovery, artifact build, install, and local execution
- each skill owns its own artifact contract and JSON schema

The examples are intentionally dependency-light adapter shapes. Keep them
focused on translating framework tool calls into `skrun.skill(...).call(...)`
rather than reimplementing the agent framework.
