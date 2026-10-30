---
title: 新增 skill kind 或命令
source: #5c0735907353
---

新增一种 kind 需要改动 `runtime.rs` 中所有对 `ArtifactKind` 的 `match`：`validate`、`scaffold_skill`、`build_skill`、`run_skill` 和 `kind_label`，以及 `main.rs` 中的 `parse_kind` 和重复的 `kind_label`。Python 端把 `kind` 当作普通字符串，除非新 kind 需要新选项，否则无需改动。保持运行时契约不变：任何可执行的东西仍然必须从 stdin 接收一个 JSON 值，并向 stdout 返回一个 JSON 对象。

runtime、CLI、绑定和 SDK 模块开头都有一段文档头（`Owns`、`Must Not`、`Inputs`、`Outputs`、`Depends On`、`Verify`）。转移职责时要更新它，并遵守 `Must Not` 列表：runtime 不做规划、不解析 `@skill` 提及、不渲染 UI，也不保存持久状态。新操作应先放进 `runtime.rs`，再通过 `main.rs` 和 `crates/py/src/lib.rs` 暴露出去，绝不在 Python 中实现。
