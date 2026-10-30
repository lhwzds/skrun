---
title: 试一个真实示例
source: #a907f45c9b82
---

`examples/skills` 中有 `rust-echo`、`python-echo`、`regex-finder`（基于 Rust `regex` crate）和 `cdp-browser`（通过 Chrome DevTools Protocol 控制浏览器）。后两个较大的示例展示了现有命令行工具如何通过添加 `artifact.json` 和 JSON schema 变成 skill，并且每个都附带一个 `smoke.json` 输入：

```bash
skrun skill build examples/skills/regex-finder
skrun skill run --input "$(cat examples/skills/regex-finder/smoke.json)" examples/skills/regex-finder
```

`regex-finder` 接收 `{"action": "match", "input": {"pattern": ..., "text": ...}}`（或 `"action": "describe"`），并返回一个包含 `ok`/`action`/`data`/`error` 的信封结构。构建会把 `bin/release/regex-finder` 写入示例目录中。
