---
title: 
order: 0
evidence:
  - Cargo.toml:1-17 #888a31f712fd
  - crates/py/Cargo.toml:13-15 #5a2c60b4863f
  - crates/py/build.rs #c39787386339
---

The Cargo workspace has two members: `crates/skrun` (default member: runtime library and CLI) and `crates/py` (`skrun-native`). PyO3 is behind the optional `python-module` feature, so `cargo build` and `cargo test` at the root never need a Python interpreter; only maturin and the native checks enable it. The module map is in [[structure]].
