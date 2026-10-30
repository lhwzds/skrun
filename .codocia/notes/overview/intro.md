---
title: 
order: 0
evidence:
  - crates/skrun/src/runtime.rs:1-37 #58a723f27bbf
  - README.md:11-19 #41e3387d83de
---

skrun is a small local runtime for *skills*: directories that package either guidance text (`SKILL.md`) or an executable program behind a fixed JSON contract. It scaffolds, builds, installs and runs those directories, so an agent framework you already use (LangChain, LangGraph, PydanticAI, OpenAI Agents) can call a Rust binary or a uv-managed Python script as an ordinary function that takes a dict and returns a dict.

It deliberately does not plan, chat, store state or manage credentials: the runtime module lists those as things it must not own. You keep your agent loop; skrun is the part that turns "a tool on disk" into "one JSON in, one JSON object out".

Start with [[getting-started]], look up exact names in [[reference]], and read [[how-it-works]] for how a call is executed.
