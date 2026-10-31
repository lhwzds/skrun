---
title: Catalog 命令
source: #ad3cae588db9
---

`skrun catalog` 处理的是一个指导性 skill 目录：含 `SKILL.md` 的非隐藏子目录，以及除 `SKILL_REGISTRY.md`、`README.md`、`README_CN.md`、`AGENTS.md` 和 `CLAUDE.md` 之外的顶层 `*.md`。

| 命令 | 参数（默认值） | 何时以 1 退出 |
|---|---|---|
| `index <dir>` | `--out <file>`（stdout） | 目录无法读取 |
| `registry <dir>` | `--registry`（`<dir>/SKILL_REGISTRY.md`）、`--write` | 不带 `--write` 时：表格有偏差；任何模式下：文件不存在、没有表格，或无法读写 |
| `check <dir>` | `--registry`、`--index` | 有任何错误 |

`check` 报为错误的有：缺少 frontmatter `name` 或 `description` 的文件夹 skill、失效的相对链接、没有表格或有偏差的注册表、与新生成索引不一致的 `skills.json`，以及看起来像私钥或 AWS、OpenAI、GitHub、Slack 令牌的行。唯一的警告是 frontmatter `name` 与目录名不同。注册表和索引在默认路径存在或显式传入时才检查。

`check` 和 `registry` 打印的用户文本（名称、链接目标、路径、格式错误的行、未知选项）若匹配上述模式，会经 `redact` 显示为 `<redacted>`；密钥发现只给出 `path:line`，路径同样会被遮蔽。`skills.json` 保留真实路径（[[how-it-works]]）。
