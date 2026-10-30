---
title: 创建、构建并运行一个 skill
source: #5b0db9bd6953
---

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
