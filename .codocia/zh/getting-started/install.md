---
title: 安装
source: #c42697622fdc
---

```bash
pip install skrun                 # 带原生扩展的 Python SDK
cargo install skrun               # 从 crates.io 安装 `skrun` CLI
cargo install --path crates/skrun # 或者：从本地检出的代码安装 CLI
```

两个包都已发布：PyPI 上是 `skrun` 0.1.0，crates.io 上是 `skrun` 0.1.1（版本为何不同见 [[contributing]]）。PyPI 发布任务运行在 `ubuntu-22.04` 和 Python 3.13 上，所以发布的二进制 wheel 只针对该平台；在其他平台上 pip 会回退到 sdist，需要 Rust 和 maturin 来编译 `skrun-native`。如果想直接基于检出的代码开发，见 [[contributing]]。
