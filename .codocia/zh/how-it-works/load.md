---
title: 加载 skill 目录
source: #4929f7aaa8f7
---

如果存在 `artifact.json`，先解码它，再由 `apply_skill_markdown` 用 `SKILL.md` 填充清单中留空的元数据字段（`description`、`tags`、`suggested_tools`、`content`、`source_ref`）。清单始终优先。如果没有 `artifact.json`，目录中必须有 `SKILL.md`，并成为一个以目录名命名的 `markdown` skill。无论哪种情况，`validate` 都在最后运行，所以每个调用方（CLI、Rust 库、Python）遇到错误清单时看到的错误都一样。
