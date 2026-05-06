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

The integration pattern is always the same:

1. Build or install a skill under a known local skill root.
2. Register a framework tool whose body calls `skrun.skill(...).call(...)`.
3. Return the skill's JSON object to the framework.
4. Keep retries, model prompts, memory, and graph transitions in the framework.

## Prepare A Skill

For local development, build and run a checked-in example skill directly:

```bash
skrun skill build examples/skills/regex-finder
skrun skill run \
  --input '{"action":"match","input":{"pattern":"TODO","text":"TODO: ship docs"}}' \
  examples/skills/regex-finder
```

For framework use, install the skill under a stable root and call it by id:

```bash
skrun skill install-local \
  --root ~/.skrun/skills \
  --overwrite \
  examples/skills/regex-finder
```

```python
import skrun

result = skrun.skill("regex-finder").call(
    {
        "action": "match",
        "input": {
            "pattern": "TODO",
            "text": "TODO: ship docs",
        },
    }
)
```

Skill ids resolve under `~/.skrun/skills` by default. Set `SKRUN_SKILLS_DIR`
when a framework process should use a workspace-local skill root:

```bash
export SKRUN_SKILLS_DIR="$PWD/.skrun/skills"
```

## Plain Tool Body

```python
import skrun


def regex_finder_tool(arguments: dict) -> dict:
    return skrun.skill("regex-finder").call(arguments)
```

The wrapper can be registered as a framework-specific tool function.

The framework should treat skrun like a local tool boundary: validate the tool
arguments, call the skill, and return the JSON result to the model or graph.

## LangChain Tool

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

In this shape, LangChain owns the model call and tool selection. skrun only runs
the installed `regex-finder` skill and returns its JSON result.

## LangGraph Node

Use skrun inside a graph node when the skill result should become part of graph
state:

```python
from typing import Any

import skrun


def regex_finder_node(state: dict[str, Any]) -> dict[str, Any]:
    result = skrun.skill("regex-finder").call(
        {
            "pattern": state["pattern"],
            "path": state.get("path", "."),
        }
    )
    return {"regex_finder_result": result}
```

LangGraph still owns graph state, routing, retries, and checkpointing. The node
only translates current state into one skill call and returns a state patch.

## OpenAI Agents-style Function Tool

When a framework expects ordinary Python functions as tool bodies, keep the
signature typed and put the skrun call inside the function:

```python
from typing import Any

import skrun


def regex_finder_function_tool(pattern: str, path: str = ".") -> dict[str, Any]:
    return skrun.skill("regex-finder").call(
        {
            "pattern": pattern,
            "path": path,
        }
    )
```

The framework can expose `pattern` and `path` to the model. The skill contract
stays JSON in and JSON out, so the same installed skill can be reused by another
framework without changing the skill implementation.

## PydanticAI-style Tool

If the framework validates tool inputs with a typed object, convert the object
to the skill's JSON input at the boundary:

```python
from dataclasses import dataclass
from typing import Any

import skrun


@dataclass
class RegexFinderArgs:
    pattern: str
    path: str = "."


def regex_finder_pydantic_tool(arguments: RegexFinderArgs) -> dict[str, Any]:
    return skrun.skill("regex-finder").call(
        {
            "pattern": arguments.pattern,
            "path": arguments.path,
        }
    )
```

Keep validation errors in the framework layer. Keep skill execution errors as
tool results or tool exceptions according to the framework's existing policy.

## Custom CLI Agent

For a custom terminal agent, the loop can stay simple: parse the model's tool
call, dispatch to skrun, then append the JSON result back into the transcript.

```python
import json
import skrun


def run_tool_call(tool_name: str, arguments_json: str) -> dict:
    arguments = json.loads(arguments_json)
    if tool_name == "regex_finder":
        return skrun.skill("regex-finder").call(arguments)
    raise ValueError(f"unknown tool: {tool_name}")
```

This keeps the CLI agent in charge of prompts, approvals, terminal rendering,
and conversation state while skrun owns the executable skill boundary.

## Checked-in Examples

Dependency-light adapter bodies live under `examples/frameworks`:

- `langchain_tool.py` shows a function body that can sit behind a LangChain
  tool wrapper.
- `langgraph_node.py` shows a graph node that returns a state patch.
- `openai_agents_tool.py` shows a typed function body for function-tool style
  frameworks.
- `pydantic_ai_tool.py` shows typed argument conversion before calling a skill.

The examples intentionally avoid importing those frameworks so skrun can test
the adapter bodies without adding every framework as a development dependency.

## Design Rule

Keep these responsibilities separate:

- the agent framework owns planning, model calls, chat state, and graph state
- skrun owns skill discovery, artifact build, install, and local execution
- each skill owns its own artifact contract and JSON schema

The examples are intentionally dependency-light adapter shapes. Keep them
focused on translating framework tool calls into `skrun.skill(...).call(...)`
rather than reimplementing the agent framework.
