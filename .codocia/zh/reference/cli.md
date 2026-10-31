---
title: CLI 命令与参数
source: #2308dfec4b73
---

skill 命令都在 `skrun skill` 下；`skrun catalog` 见下文。位置参数 `<dir>` / `<id>` 可以出现在参数之间的任意位置。

| 命令 | 参数（默认值） |
|---|---|
| `new <dir>` | `--kind markdown\|guidance\|rust_binary\|python_uv`（必填）、`--id`（必填）、`--name`（id 转为首字母大写：`regex-finder` → `Regex Finder`）、`--version`（`0.1.0`） |
| `build <dir>` | `--profile`（`release`）、`--target-dir`、`--cargo`（`cargo`）、`--uv`（`uv`） |
| `run <dir>` 或 `run --id <id>` | `--input`（`{}`；必须是 JSON 对象）、`--timeout` 秒数（`60`）、`--uv`（`uv`）、`--root`（仅配合 `--id`） |
| `install-local <dir>` | `--root`、`--id`（安装时重命名）、`--overwrite` |
| `list` | `--root`、`--format text\|json`（`text`） |
| `show <id>` | `--root`、`--format text\|json`（`text`） |

`--root` 默认为 skill 根目录（见下文）。`list` 和 `show` 打印以制表符分隔的 `id kind version name`。`run` 不接受同时给出目录和 `--id`。`--cargo` 和 `--uv` 没有写进用法说明。出错时向 stderr 打印 `Error: ...` 并以 1 退出，用法错误（未知、缺失或无效的参数）则以 2 退出；看起来像密钥的回显参数和路径显示为 `<redacted>`。
