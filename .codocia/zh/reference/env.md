---
title: 环境变量与默认路径
source: #3fe04275c687
---

| 名称 | 作用 |
|---|---|
| `SKRUN_SKILLS_DIR` | `run --id`、`install-local`、`list`、`show` 和 Python SDK 使用的 skill 根目录 |
| `HOME` | 后备根目录 `$HOME/.skrun/skills`；两者都未设置时，CLI 和 Rust 调用方会以 `HOME is not set` 失败 |
| `SKRUN_STORE` | 未给 `--store` 时 `catalog add` 和 `inspect` 使用的存储，否则为 `$HOME/.skrun/store`（[[store]]） |

CLI 会提前计算这个默认值（`root.unwrap_or(default_skills_dir()?)`），所以两个变量都未设置时，即使传了 `--root`，`run --id`、`install-local`、`list` 和 `show` 也会以 `HOME is not set` 失败。

`build` 构建 `rust_binary` 时，除非指定 `--target-dir`，否则使用 `<system temp dir>/skrun-skill-targets/<id>` 作为 Cargo 的 target 目录，因此 skill 构建不会在 skill 目录中留下 Cargo target 目录。Python SDK 自行解析同样的根目录（会展开 `~`），设置了 `root=` 参数时则直接传递该参数。
