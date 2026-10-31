---
title: Catalog 命令
source: #ba66b5be5baf
---

`skrun catalog` 处理的是一个指导性 skill 目录，而不是单个制品：每个含 `SKILL.md` 的非隐藏子目录，以及除 `SKILL_REGISTRY.md`、`README.md`、`README_CN.md`、`AGENTS.md` 和 `CLAUDE.md` 之外的每个顶层 `*.md`。

| 命令 | 参数（默认值） | 何时以 1 退出 |
|---|---|---|
| `index <dir>` | `--out <file>`（stdout） | 目录无法读取 |
| `registry <dir>` | `--registry`（`<dir>/SKILL_REGISTRY.md`）、`--write` | 不带 `--write` 时：表格有偏差；两种模式下：文件不存在、没有表格，或无法读写（`--write` 修复后以 0 退出） |
| `check <dir>` | `--registry`、`--index` | 有任何错误 |

`check` 报为错误的有：缺少 frontmatter `name` 或 `description` 的文件夹 skill、失效的相对链接、没有表格或有偏差的注册表、与新生成索引不一致的 `skills.json`，以及看起来像私钥或 AWS、OpenAI、GitHub、Slack 令牌的行（只打印 `path:line`，从不打印值）。唯一的警告是 frontmatter `name` 与目录名不同。注册表只在文件存在或给出 `--registry` 时检查；索引只在 `<dir>/skills.json` 存在或给出 `--index` 时检查。[[how-it-works]] 解释了输出为何是确定性的。
