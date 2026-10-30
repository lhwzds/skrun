---
title: 本地开发流程
source: #7d9dc614166c
---

```bash
make test     # cargo test + python/tests 的 python unittest
make lint     # cargo fmt --check + clippy -D warnings
make python   # scripts/check.sh ci：完整打包流程
make check    # lint + test + python
```

`scripts/check.sh ci` 会创建 venv、安装 maturin、运行 Python 契约测试、用 `--features python-module` 测试并 lint `skrun-native`，然后通过 `python/scripts/package.py` 依次执行 `develop`、`smoke`、`build` 和 `sdist`。每一步也都是一个子命令（`bootstrap`、`python`、`rust`、`develop`、`smoke`、`build`、`sdist`）。它读取 `PYTHON_BIN`（`python3`）、`PYTHON_VENV`（`/tmp/skrun-python-venv`）、`CARGO_TARGET_DIR`（`/tmp/skrun-python-target`）和 `WHEEL_DIR`（`/tmp/skrun-python-dist`）。`package.py` 会自行设置 `PYO3_PYTHON` 和 `CARGO_TARGET_DIR`；默认 target 放在 `/tmp`，这样原生产物不会在外接 macOS 卷上构建或执行。`make clean` 会删除这三个 `/tmp` 路径。
