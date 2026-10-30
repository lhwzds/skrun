---
title: 从 agent 框架调用 skill
source: #45ded9ecc96c
---

skrun 没有任何框架适配器；工具函数体只需一行，工具 schema 由框架自己的装饰器或包装器提供：

```python
import skrun

def regex_finder_tool(pattern: str, text: str) -> dict:
    return skrun.skill("regex-finder").call(
        {"action": "match", "input": {"pattern": pattern, "text": text}}
    )
```

`examples/frameworks` 中为 LangChain、LangGraph、PydanticAI 和 OpenAI Agents 提供了相同结构的示例。LangChain 示例把工具的 `arguments` dict 原样传入。另外三个构造的是 `{"pattern": ..., "path": ...}`，而 `regex-finder` 要求 `text`，所以会拒绝；请使用上面的输入结构。每个 skill 中的 `schema/input.json` 天然适合作为框架参数 schema 的来源。错误以 `SkillRuntimeError` 的形式抛出，大多数框架会把它反馈给模型。
