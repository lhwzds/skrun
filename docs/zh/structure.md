---
title: 代码结构
---

# 代码结构

<sub>[English](../structure.md) · **中文**</sub>

本页由 Codocia 根据代码自动生成，每次 build 重新计算：功能区划分、各区自带的说明、区之间的 import 依赖，以及讲解各区的页面。

范围内共 53 个文件，16,537 行；已排除 `docs/**`、`python/tests/**`、`**/tests/**`、`Cargo.lock`、`.github/dependabot.yml`、`examples/skills/cdp-browser/src/**`、`.gitattributes`、`.gitignore`、`LICENSE`

## 文件结构

**仓库根目录**

- （根目录文件） — 466 行，5 个文件。skrun executable skill runtime internals.\
  讲解：[概览](01-overview.md) (2), [快速上手](02-getting-started.md) (1), [参与开发](06-contributing.md) (3)
- `.github/` — 313 行，4 个文件\
  讲解：[快速上手](02-getting-started.md) (2), [参与开发](06-contributing.md) (1)
- `python/` — 548 行，4 个文件。`skrun` is the Python SDK for the skrun executable skill runtime.\
  核心文件：`skrun/skill.py` `scripts/package.py` `skrun/__init__.py`\
  讲解：[概览](01-overview.md) (1), [快速上手](02-getting-started.md) (2), [参考](03-reference.md) (2), [工作原理](04-how-it-works.md) (1), [参与开发](06-contributing.md) (3)
- `scripts/` — 124 行，1 个文件\
  核心文件：`check.sh`\
  讲解：[参与开发](06-contributing.md) (1)

**`crates/`**

- `py/` — 269 行，3 个文件。PyO3 module boundary for the skrun executable skill runtime.\
  核心文件：`src/lib.rs` `build.rs`\
  讲解：[概览](01-overview.md) (1), [工作原理](04-how-it-works.md) (1), [参与开发](06-contributing.md) (1)

**`crates/skrun/`**

- （直接文件） — 23 行，1 个文件。Public API for the skrun executable skill runtime.\
  讲解：[快速上手](02-getting-started.md) (1)
- `src/` — 14,113 行，6 个文件。Public API for the skrun executable skill runtime.\
  核心文件：`catalog.rs` `store.rs` `runtime.rs`\
  讲解：[概览](01-overview.md) (3), [快速上手](02-getting-started.md) (3), [参考](03-reference.md) (7), [工作原理](04-how-it-works.md) (8), [存储与风险等级](05-store.md) (8), [参与开发](06-contributing.md) (1)

**`examples/`**

- `frameworks/` — 92 行，4 个文件\
  核心文件：`pydantic_ai_tool.py` `langgraph_node.py` `langchain_tool.py`\
  讲解：[快速上手](02-getting-started.md) (1)
- `skills/` — 589 行，25 个文件\
  核心文件：`regex-finder/src/main.rs` `python-echo/skill.py` `rust-echo/src/main.rs`\
  讲解：[快速上手](02-getting-started.md) (2), [工作原理](04-how-it-works.md) (1)

