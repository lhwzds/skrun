---
title: Install
order: 10
evidence:
  - crates/skrun/Cargo.toml:1-15 #4262d595fba8
  - .github/workflows/publish-python.yml:22 #a4780650d3f8
  - .github/workflows/publish-python.yml:34-37 #7bef1cab1dbe
---

```bash
pip install skrun                 # Python SDK with the native extension
cargo install skrun               # the `skrun` CLI from crates.io
cargo install --path crates/skrun # or: the CLI from a checkout
```

Both packages are published: PyPI has `skrun` 0.1.0 and crates.io has `skrun` 0.1.1 (why the versions differ: [[contributing]]). The PyPI job runs on `ubuntu-22.04` with Python 3.13, so the published binary wheel targets that platform; elsewhere pip falls back to the sdist and needs Rust plus maturin to compile `skrun-native`. To work from a checkout instead, see [[contributing]].
