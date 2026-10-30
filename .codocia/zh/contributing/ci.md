---
title: CI 与发布
source: #907ce2d83b3d
---

| workflow | 触发条件 | 做什么 |
|---|---|---|
| `python.yml` | 改动代码的、针对 `main` 的 push / PR | 执行 `check.sh` 的各步骤，上传 wheel |
| `publish-crates.yml` | 标签 `v*` 或手动触发 | `cargo test -p skrun`、dry-run、`cargo publish -p skrun` |
| `publish-python.yml` | 标签 `v*` 或手动触发 | `scripts/check.sh ci`，然后发布到 PyPI（`skip-existing`） |
| `release-skills.yml` | 标签 `<skill>@<version>` 或手动触发 | 构建、冒烟运行，并把 `<skill>@<version>-x86_64-linux.tar.gz`（`artifact.json`、`SKILL.md`、`schema`、`bin`）附加到 GitHub release |

`release-skills.yml` 只接受 `regex-finder` 和 `cdp-browser`，并用 CLI 本身来构建每个示例并运行它的 `smoke.json`。crates.io 和 PyPI 的发布都使用可信发布（`id-token: write`），而不是存储的 token。各处版本号分开设置：工作区 `Cargo.toml` 写的是 0.1.1（已发布的 crate），而 `pyproject.toml` 和 `python/skrun/__init__.py` 写的是 0.1.0（已发布的 PyPI 包）。发布时三处都要更新；由于 `skip-existing`，没有更新版本号的 Python 包会被静默跳过，不会重新上传。
