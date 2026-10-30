---
title: 
source: #faa485917554
---

Cargo 工作区有两个成员：`crates/skrun`（默认成员：运行时库和 CLI）和 `crates/py`（`skrun-native`）。PyO3 放在可选的 `python-module` feature 之后，所以在根目录执行 `cargo build` 和 `cargo test` 从不需要 Python 解释器；只有 maturin 和原生检查会启用它。模块划分见 [[structure]]。
