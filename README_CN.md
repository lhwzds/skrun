# skrun

[English](README.md) | 中文

skrun 是一个以 Python 为先的可执行 skill 运行时，让 agent 框架能够发现、构建、安装和调用本地 skill。

文档维护在 [`docs/`](./docs/zh/README.md) 中。发布版本和源码可从 [GitHub 仓库](https://github.com/lhwzds/skrun)获取。

## 原则

- 模块名尽量短。
- 每个模块只承担一项明确的职责。
- 运行时语义与 UI 交互分离。
- 可执行 skill 运行时不负责持久化存储。
- Python 绑定与 Rust API 保持相同的模块边界。
- 支持外部 agent 框架，而不是取代它们的主 agent 循环。
- Rust 二进制 skill 和基于 uv 的 Python skill 都是一等公民。

## 模块

```text
crates/skrun  runtime: artifacts, scaffold, build, install, and run; plus the skrun CLI
crates/py     skrun-native PyO3 module used by the Python package
python/skrun  Python SDK over skrun-native
```

## Skill

skrun 支持纯指导性的 Markdown skill 和可执行制品：

```text
markdown     SKILL.md-only guidance skill, listed and shown but not executed
rust_binary  Cargo-built executable skill, called with stdin/stdout JSON
python_uv    uv-managed Python project, called with stdin/stdout JSON
```

可执行 skill 目录中包含一个 `artifact.json` 清单：

```json
{
  "schema_version": 1,
  "kind": "rust_binary",
  "id": "regex-finder",
  "name": "Regex Finder",
  "version": "0.1.0",
  "entry": "bin/release/regex-finder",
  "protocol": {
    "transport": "stdio-json",
    "input": "single-json-value",
    "output": "single-json-value"
  },
  "schema": {
    "input": "schema/input.json",
    "output": "schema/output.json"
  }
}
```

初始协议有意定得很严格：

- 输入是写入 stdin 的一个 JSON 值
- 输出是写入 stdout 的一个 JSON 对象
- stderr 用于诊断信息
- 非零退出码即失败
- 流式输出不属于第一版可执行 skill 契约

## Python 打包流程

PyPI 发行包名和 Python 导入名都是 `skrun`。Python 包底层由 Rust 的 `skrun-native` PyO3 模块支撑。请使用打包辅助脚本，让本地安装和发布 wheel 使用相同的设置：

```bash
python3 -m pip install maturin
python3 python/scripts/package.py develop
python3 python/scripts/package.py smoke
python3 python/scripts/package.py build
python3 python/scripts/package.py sdist
```

该脚本会设置 `PYO3_PYTHON`，并默认把 Cargo 产物放在 `/tmp/skrun-python-target` 下，避免从外接 macOS 卷上执行 Rust 构建产物。

## Python Skill API

高层 Python API 通过与 CLI 相同的 Rust `runtime` crate 调用本地可执行 skill 制品：

```python
import skrun

result = skrun.skill("/path/to/skill").call({"action": "match", "input": {"pattern": "TODO", "text": "TODO: ship it"}})
```

默认情况下，skill ID 在 `~/.skrun/skills` 下解析，或在 `SKRUN_SKILLS_DIR` 指定的目录下解析。Python 包执行可执行 skill 操作时依赖 `skrun_native` PyO3 扩展，不另外维护基于子进程的运行时后备方案。

## CLI Skill 流程

这个极简 CLI 可以生成指导性或可执行 skill 的脚手架：

```bash
cargo run -p skrun -- \
  skill new --kind markdown --id team /tmp/team

cargo run -p skrun -- \
  skill new --kind rust_binary --id rust-echo /tmp/rust-echo

cargo run -p skrun -- \
  skill build /tmp/rust-echo

cargo run -p skrun -- \
  skill run --input '{"ok":true}' /tmp/rust-echo

cargo run -p skrun -- \
  skill install-local --root /tmp/skrun-skills --overwrite /tmp/rust-echo

cargo run -p skrun -- \
  skill list --root /tmp/skrun-skills --format json

cargo run -p skrun -- \
  skill show --root /tmp/skrun-skills --format json rust-echo
```

示例 skill 位于 `examples/skills` 下：

```text
rust-echo      minimal Rust stdio-json echo skill
python-echo    minimal uv-backed Python stdio-json echo skill
regex-finder   Rust binary regex matching skill
cdp-browser    CDP browser-control binary skill
```

较复杂的二进制示例展示了如何给现有的 stdin/stdout 工具加上 `artifact.json` 和 JSON schema 文件，把它变成 skrun 制品。

## Skill 仓库目录

`skrun catalog` 维护一个指导性 skill 目录：含 `SKILL.md` 的子目录，以及顶层的 `*.md` 文件（`SKILL_REGISTRY.md`、README、`AGENTS.md`、`CLAUDE.md` 除外）。它从不运行 skill，也不调用 git 或网络。

```bash
# 写出确定性的 skills.json 索引（不带 --out 时输出到 stdout）
cargo run -p skrun -- catalog index skills --out skills/skills.json

# 报告 SKILL_REGISTRY.md 的偏差；不同步或缺少表格时以 1 退出
cargo run -p skrun -- catalog registry skills

# 删除失效/重复的行，为未登记的 skill 追加行；同步后以 0 退出
cargo run -p skrun -- catalog registry skills --write

# 检查 frontmatter、链接、注册表、过期索引和疑似密钥；有错误时以 1 退出
cargo run -p skrun -- catalog check skills
```

## Agent 框架示例

skrun 不取代主 agent 框架。用你已经在用的框架包装可执行 skill 即可：

```python
import skrun


def regex_finder_tool(arguments: dict) -> dict:
    return skrun.skill("regex-finder").call(arguments)
```

`examples/frameworks` 下有依赖很轻的示例，覆盖 LangChain、LangGraph、PydanticAI 以及 OpenAI Agents 风格的工具函数体。

## 本阶段的非目标

- 不实现主 agent 框架。
- 不实现远程市场 UI。
- 不实现流式可执行 skill。
