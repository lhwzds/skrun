---
title: 参与开发
---

# 参与开发

<sub>[English](../05-contributing.md) · **中文**</sub>

Cargo 工作区有两个成员：`crates/skrun`（默认成员：运行时库和 CLI）和 `crates/py`（`skrun-native`）。PyO3 放在可选的 `python-module` feature 之后，所以在根目录执行 `cargo build` 和 `cargo test` 从不需要 Python 解释器；只有 maturin 和原生检查会启用它。模块划分见 [代码结构](structure.md)。

<sub>来源: [Cargo.toml:1-17](../../Cargo.toml#L1-L17) · [Cargo.toml:13-15](../../crates/py/Cargo.toml#L13-L15) · [build.rs](../../crates/py/build.rs)</sub>

## 本地开发流程

```bash
make test     # cargo test + python/tests 的 python unittest
make lint     # cargo fmt --check + clippy -D warnings
make python   # scripts/check.sh ci：完整打包流程
make check    # lint + test + python
```

`scripts/check.sh ci` 会创建 venv、安装 maturin、运行 Python 契约测试、用 `--features python-module` 测试并 lint `skrun-native`，然后通过 `python/scripts/package.py` 依次执行 `develop`、`smoke`、`build` 和 `sdist`。每一步也都是一个子命令（`bootstrap`、`python`、`rust`、`develop`、`smoke`、`build`、`sdist`）。它读取 `PYTHON_BIN`（`python3`）、`PYTHON_VENV`（`/tmp/skrun-python-venv`）、`CARGO_TARGET_DIR`（`/tmp/skrun-python-target`）和 `WHEEL_DIR`（`/tmp/skrun-python-dist`）。`package.py` 会自行设置 `PYO3_PYTHON` 和 `CARGO_TARGET_DIR`；默认 target 放在 `/tmp`，这样原生产物不会在外接 macOS 卷上构建或执行。`make clean` 会删除这三个 `/tmp` 路径。

<sub>来源: [Makefile](../../Makefile) · [check.sh:7-13](../../scripts/check.sh#L7-L13) · [check.sh:59-89](../../scripts/check.sh#L59-L89) · [package.py:13-41](../../python/scripts/package.py#L13-L41)</sub>

## 冒烟测试验证了什么

`package.py smoke` 在 `/tmp` 下运行，因此导入的是已安装的包而不是源码树。它会写一个临时的 `rust_binary` skill，其 entry 是一个运行 `cat` 的 `#!/bin/sh` 脚本，然后断言 `skrun.skill(root).call({'native': True})` 返回相同的 dict。用 shell 脚本就够了，因为 `rust_binary` 只要求 `entry` 处有一个可执行文件；这也让它成为在测试中包装任意现有可执行文件的快捷方法。

<sub>来源: [package.py:64-88](../../python/scripts/package.py#L64-L88)</sub>

## CI 与发布

| workflow | 触发条件 | 做什么 |
|---|---|---|
| `python.yml` | 改动代码的、针对 `main` 的 push / PR | 执行 `check.sh` 的各步骤，上传 wheel |
| `publish-crates.yml` | 标签 `v*` 或手动触发 | `cargo test -p skrun`、dry-run、`cargo publish -p skrun` |
| `publish-python.yml` | 标签 `v*` 或手动触发 | `scripts/check.sh ci`，然后发布到 PyPI（`skip-existing`） |
| `release-skills.yml` | 标签 `<skill>@<version>` 或手动触发 | 构建、冒烟运行，并把 `<skill>@<version>-x86_64-linux.tar.gz`（`artifact.json`、`SKILL.md`、`schema`、`bin`）附加到 GitHub release |

`release-skills.yml` 只接受 `regex-finder` 和 `cdp-browser`，并用 CLI 本身来构建每个示例并运行它的 `smoke.json`。crates.io 和 PyPI 的发布都使用可信发布（`id-token: write`），而不是存储的 token。各处版本号分开设置：工作区 `Cargo.toml` 写的是 0.1.1（已发布的 crate），而 `pyproject.toml` 和 `python/skrun/__init__.py` 写的是 0.1.0（已发布的 PyPI 包）。发布时三处都要更新；由于 `skip-existing`，没有更新版本号的 Python 包会被静默跳过，不会重新上传。

<sub>来源: [python.yml:1-20](../../.github/workflows/python.yml#L1-L20) · [publish-crates.yml:1-52](../../.github/workflows/publish-crates.yml#L1-L52) · [publish-python.yml:1-11](../../.github/workflows/publish-python.yml#L1-L11) · [publish-python.yml:50-57](../../.github/workflows/publish-python.yml#L50-L57) · [release-skills.yml:1-103](../../.github/workflows/release-skills.yml#L1-L103) · +3</sub>

## 新增 skill kind 或命令

新增一种 kind 需要改动 `runtime.rs` 中所有对 `ArtifactKind` 的 `match`：`validate`、`scaffold_skill`、`build_skill`、`run_skill` 和 `kind_label`，以及 `main.rs` 中的 `parse_kind` 和重复的 `kind_label`。Python 端把 `kind` 当作普通字符串，除非新 kind 需要新选项，否则无需改动。保持运行时契约不变：任何可执行的东西仍然必须从 stdin 接收一个 JSON 值，并向 stdout 返回一个 JSON 对象。

runtime、CLI、绑定和 SDK 模块开头都有一段文档头（`Owns`、`Must Not`、`Inputs`、`Outputs`、`Depends On`、`Verify`）。转移职责时要更新它，并遵守 `Must Not` 列表：runtime 不做规划、不解析 `@skill` 提及、不渲染 UI，也不保存持久状态。新操作应先放进 `runtime.rs`，再通过 `main.rs` 和 `crates/py/src/lib.rs` 暴露出去，绝不在 Python 中实现。

<sub>来源: [runtime.rs:55-61](../../crates/skrun/src/runtime.rs#L55-L61) · [runtime.rs:586-592](../../crates/skrun/src/runtime.rs#L586-L592) · [main.rs:577-606](../../crates/skrun/src/main.rs#L577-L606) · [runtime.rs:3-17](../../crates/skrun/src/runtime.rs#L3-L17)</sub>
