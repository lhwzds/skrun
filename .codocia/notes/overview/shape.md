---
title: One runtime, three front doors
order: 20
evidence:
  - crates/skrun/src/lib.rs:1-14 #dfbf2e563d16
  - crates/py/src/lib.rs:103-129 #2a80d2aafa50
  - python/skrun/skill.py:295-321 #d44c8526b13b
  - pyproject.toml:38-44 #4ef68d3d3b89
---

Skill behaviour lives in one Rust module, `crates/skrun/src/runtime.rs`; a second module, `catalog`, only does bookkeeping for a directory of guidance skills ([[reference]]). The `skrun` crate re-exports the runtime as a library and also ships the `skrun` CLI binary. Python reaches the same code through `skrun-native` (`crates/py`), a PyO3 extension that maturin installs as `skrun.skrun_native`; the pure-Python `skrun` package only converts arguments and JSON.

```mermaid
flowchart LR
  cli["skrun CLI<br/>crates/skrun/src/main.rs"] --> rt["runtime.rs"]
  lib["Rust callers<br/>use skrun::*"] --> rt
  py["Python: import skrun<br/>python/skrun/skill.py"] --> native["skrun.skrun_native<br/>crates/py/src/lib.rs"] --> rt
  rt --> proc["skill process<br/>stdin JSON -> stdout JSON"]
```

Because the Python package has no subprocess fallback, a Python call and a CLI call validate and execute a skill identically.
