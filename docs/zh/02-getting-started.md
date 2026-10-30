---
title: 快速上手
---

# 快速上手

<sub>[English](../02-getting-started.md) · **中文**</sub>

构建 `rust_binary` skill 需要 Rust 工具链，构建和运行 `python_uv` skill 需要 [uv](https://docs.astral.sh/uv/)，使用 Python 包需要 Python 3.11 或更高版本。skrun 默认直接调用 `cargo` 和 `uv` 命令，所以两者都必须在 PATH 中。

<sub>来源: [runtime.rs:313-337](../../crates/skrun/src/runtime.rs#L313-L337) · [pyproject.toml:6-8](../../pyproject.toml#L6-L8)</sub>

## 安装

```bash
pip install skrun                 # 带原生扩展的 Python SDK
cargo install skrun               # 从 crates.io 安装 `skrun` CLI
cargo install --path crates/skrun # 或者：从本地检出的代码安装 CLI
```

两个包都已发布：PyPI 上是 `skrun` 0.1.0，crates.io 上是 `skrun` 0.1.1（版本为何不同见 [参与开发](05-contributing.md)）。PyPI 发布任务运行在 `ubuntu-22.04` 和 Python 3.13 上，所以发布的二进制 wheel 只针对该平台；在其他平台上 pip 会回退到 sdist，需要 Rust 和 maturin 来编译 `skrun-native`。如果想直接基于检出的代码开发，见 [参与开发](05-contributing.md)。

<sub>来源: [Cargo.toml:1-15](../../crates/skrun/Cargo.toml#L1-L15) · [publish-python.yml:22](../../.github/workflows/publish-python.yml#L22) · [publish-python.yml:34-37](../../.github/workflows/publish-python.yml#L34-L37)</sub>

## 创建、构建并运行一个 skill

```bash
skrun skill new --kind rust_binary --id rust-echo /tmp/rust-echo
skrun skill build /tmp/rust-echo
skrun skill run --input '{"ok":true}' /tmp/rust-echo
# {
#   "ok": true
# }
```

`new` 会生成一个独立的 Cargo 项目（`Cargo.toml` 中带一个空的 `[workspace]`，`src/main.rs` 原样回显 stdin）、不做限制的 `schema/input.json` 和 `schema/output.json`、一个 `SKILL.md`，以及 `entry: bin/release/rust-echo` 的 `artifact.json`。`build` 运行 Cargo，并把二进制复制到这个 `entry` 路径。`run` 把输入通过管道传给二进制，并格式化输出它返回的 JSON 对象。

Python 则使用 `--kind python_uv`：脚手架是一个 `pyproject.toml` 加一个回显输入的 `skill.py`，`build` 会运行 `uv lock`/`uv sync`。

<sub>来源: [runtime.rs:623-659](../../crates/skrun/src/runtime.rs#L623-L659) · [runtime.rs:828-866](../../crates/skrun/src/runtime.rs#L828-L866) · [runtime.rs:954-992](../../crates/skrun/src/runtime.rs#L954-L992) · [main.rs:73-81](../../crates/skrun/src/main.rs#L73-L81)</sub>

## 安装 skill 并从 Python 调用

`install-local` 会把 skill 目录复制到 skill 根目录（默认是 `~/.skrun/skills`，见 [参考](03-reference.md)）：

```bash
skrun skill install-local /tmp/rust-echo
skrun skill list
skrun skill run --id rust-echo --input '{"ok":true}'
```

```python
import skrun

skrun.skill("rust-echo").call({"ok": True})        # 按 id，从 skill 根目录查找
skrun.skill("/tmp/rust-echo").call({"ok": True})   # 或者按路径
```

如果参数对应的目录中包含 `artifact.json` 或 `SKILL.md`，`skrun.skill` 就把它当作路径，否则当作 skill 根目录下的 id。由于安装是复制，重新构建 skill 后要用 `--overwrite` 重新安装（[工作原理](04-how-it-works.md)）。

<sub>来源: [main.rs:82-107](../../crates/skrun/src/main.rs#L82-L107) · [skill.py:236-240](../../python/skrun/skill.py#L236-L240)</sub>

## 试一个真实示例

`examples/skills` 中有 `rust-echo`、`python-echo`、`regex-finder`（基于 Rust `regex` crate）和 `cdp-browser`（通过 Chrome DevTools Protocol 控制浏览器）。后两个较大的示例展示了现有命令行工具如何通过添加 `artifact.json` 和 JSON schema 变成 skill，并且每个都附带一个 `smoke.json` 输入：

```bash
skrun skill build examples/skills/regex-finder
skrun skill run --input "$(cat examples/skills/regex-finder/smoke.json)" examples/skills/regex-finder
```

`regex-finder` 接收 `{"action": "match", "input": {"pattern": ..., "text": ...}}`（或 `"action": "describe"`），并返回一个包含 `ok`/`action`/`data`/`error` 的信封结构。构建会把 `bin/release/regex-finder` 写入示例目录中。

<sub>来源: [artifact.json](../../examples/skills/regex-finder/artifact.json) · [smoke.json](../../examples/skills/regex-finder/smoke.json) · [main.rs:91-117](../../examples/skills/regex-finder/src/main.rs#L91-L117) · [release-skills.yml:75-82](../../.github/workflows/release-skills.yml#L75-L82)</sub>

## 从 agent 框架调用 skill

skrun 没有任何框架适配器；工具函数体只需一行，工具 schema 由框架自己的装饰器或包装器提供：

```python
import skrun

def regex_finder_tool(pattern: str, text: str) -> dict:
    return skrun.skill("regex-finder").call(
        {"action": "match", "input": {"pattern": pattern, "text": text}}
    )
```

`examples/frameworks` 中为 LangChain、LangGraph、PydanticAI 和 OpenAI Agents 提供了相同结构的示例。LangChain 示例把工具的 `arguments` dict 原样传入。另外三个构造的是 `{"pattern": ..., "path": ...}`，而 `regex-finder` 要求 `text`，所以会拒绝；请使用上面的输入结构。每个 skill 中的 `schema/input.json` 天然适合作为框架参数 schema 的来源。错误以 `SkillRuntimeError` 的形式抛出，大多数框架会把它反馈给模型。

<sub>来源: [langchain_tool.py:15-18](../../examples/frameworks/langchain_tool.py#L15-L18) · [langgraph_node.py:10-19](../../examples/frameworks/langgraph_node.py#L10-L19) · [main.rs:91-117](../../examples/skills/regex-finder/src/main.rs#L91-L117) · [input.json](../../examples/skills/regex-finder/schema/input.json) · [skill.py:172-173](../../python/skrun/skill.py#L172-L173) · +2</sub>
