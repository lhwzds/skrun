---
title: Local development loop
order: 10
evidence:
  - Makefile #b53d1ac14d16
  - scripts/check.sh:7-13 #6ecc242a87f8
  - scripts/check.sh:59-89 #26065003fd36
  - python/scripts/package.py:13-41 #55cf5769775b
---

```bash
make test     # cargo test + python unittest of python/tests
make lint     # cargo fmt --check + clippy -D warnings
make python   # scripts/check.sh ci: full package loop
make check    # lint + test + python
```

`scripts/check.sh ci` creates a venv, installs maturin, runs the Python contract tests, tests and lints `skrun-native` with `--features python-module`, then `develop`, `smoke`, `build` and `sdist` through `python/scripts/package.py`. Each step is also a subcommand (`bootstrap`, `python`, `rust`, `develop`, `smoke`, `build`, `sdist`). It reads `PYTHON_BIN` (`python3`), `PYTHON_VENV` (`/tmp/skrun-python-venv`), `CARGO_TARGET_DIR` (`/tmp/skrun-python-target`) and `WHEEL_DIR` (`/tmp/skrun-python-dist`). `package.py` sets `PYO3_PYTHON` and `CARGO_TARGET_DIR` itself; the default target lives in `/tmp` so native artifacts are not built or executed from an external macOS volume. `make clean` removes those three `/tmp` paths.
