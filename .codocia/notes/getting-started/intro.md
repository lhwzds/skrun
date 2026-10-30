---
title: 
order: 0
evidence:
  - crates/skrun/src/runtime.rs:313-337 #e781ef9878e1
  - pyproject.toml:6-8 #c995273fa1d6
---

You need a Rust toolchain to build `rust_binary` skills, [uv](https://docs.astral.sh/uv/) to build and run `python_uv` skills, and Python 3.11 or newer for the Python package. By default skrun invokes the plain commands `cargo` and `uv`, so both must be on your PATH.
