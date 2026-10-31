---
title: Catalog 命令
source: #3743cb59fa2f
---

`skrun catalog` 处理的是一个指导性 skill 目录：含 `SKILL.md` 的非隐藏子目录，以及除 `SKILL_REGISTRY.md`、`README.md`、`README_CN.md`、`AGENTS.md` 和 `CLAUDE.md` 之外的顶层 `*.md`。其中 `add` 和 `inspect` 处理的是存储（[[store]]）。

| 命令 | 参数（默认值） | 何时以 1 退出 |
|---|---|---|
| `index <dir>` | `--out <file>`（stdout） | 目录无法读取 |
| `registry <dir>` | `--registry`（`<dir>/SKILL_REGISTRY.md`）、`--write` | 不带 `--write` 时：表格有偏差；任何模式下：文件不存在、没有表格，或无法读写 |
| `check <dir>` | `--registry`、`--index` | 有任何错误 |
| `search <dir> <query>...` | `--kind folder\|flat`、`--has-scripts`、`--limit`（`10`）、`--json` | 没有匹配（`--json` 打印 `[]`） |

`check` 报为错误的有：缺少 frontmatter `name` 或 `description` 的文件夹 skill、失效的相对链接、缺失或有偏差的注册表表格、过期的 `skills.json`，以及含密钥的行。唯一的警告是 frontmatter `name` 与目录名不同。

密钥指私钥块、以 `AKIA`、`sk-`（含 `sk-proj-`、`sk-ant-`）、`gh[pousr]_`、`github_pat_`、`glpat-`、`xox[baprs]-` 或 `AIza` 开头的令牌，以及 `password: …` 这类通用赋值（[[store]]）。`check`、`registry` 和 `search` 打印的用户文本若匹配，会显示为 `<redacted>`（`redact`）；密钥发现只给出 `path:line`。`skills.json` 保留真实路径（[[how-it-works]]）。
