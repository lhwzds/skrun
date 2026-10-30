---
title: Call a skill from an agent framework
order: 50
evidence:
  - examples/frameworks/langchain_tool.py:15-18 #be00a98a090c
  - examples/frameworks/langgraph_node.py:10-19 #046e35642526
  - examples/skills/regex-finder/src/main.rs:91-117 #81b494aa100d
  - examples/skills/regex-finder/schema/input.json #af4f28475301
  - python/skrun/skill.py:172-173 #93e746a83f04
  - examples/frameworks/openai_agents_tool.py:10-18 #41d029c182c7
  - examples/frameworks/pydantic_ai_tool.py:17-25 #d96f9e45383f
---

skrun has no framework adapters; a tool body is one line, and the framework's own decorator or wrapper supplies the tool schema:

```python
import skrun

def regex_finder_tool(pattern: str, text: str) -> dict:
    return skrun.skill("regex-finder").call(
        {"action": "match", "input": {"pattern": pattern, "text": text}}
    )
```

`examples/frameworks` has the same shape for LangChain, LangGraph, PydanticAI and OpenAI Agents. The LangChain example passes the tool's `arguments` dict through unchanged. The other three build `{"pattern": ..., "path": ...}`, which `regex-finder` rejects because it requires `text`; use the input shape above. `schema/input.json` in each skill is the natural source for the framework's parameter schema. Errors surface as `SkillRuntimeError`, which most frameworks report back to the model.
