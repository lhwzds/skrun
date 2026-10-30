---
title: 构建
source: #54e7ec790bb4
---

`rust_binary`：运行 `cargo build --manifest-path <dir>/Cargo.toml --target-dir <target>`，profile 为 `release` 时加 `--release`，否则加 `--profile <name>`，然后把 `<target>/<dir>/<id>`（加上平台的可执行文件后缀）复制到 `entry`。其中 `<dir>` 是 Cargo 的输出目录：`dev` 和 `test` 为 `debug`，`release` 和 `bench` 为 `release`，其他情况为 profile 名。因此二进制名必须与 skill id 相同。无论 profile 是什么都会复制到 `entry`，所以使用脚手架生成的 entry 时，debug 构建的产物也会落在 `bin/release/...`。

`python_uv`：仅在缺少 `uv.lock` 时运行 `uv lock --project <dir>`，然后运行 `uv sync --project <dir> --locked`。锁文件是 skill 的一部分，因此安装可复现。

两种构建步骤都会捕获输出，失败时只报告 stderr。`markdown` skill 无法构建。
