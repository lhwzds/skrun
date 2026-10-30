---
title: CI and releases
order: 30
evidence:
  - .github/workflows/python.yml:1-20 #1dc34fce74aa
  - .github/workflows/publish-crates.yml:1-52 #5a829d4e2c44
  - .github/workflows/publish-python.yml:1-11 #9f74e1e74ecb
  - .github/workflows/publish-python.yml:50-57 #4f1b551d8e6b
  - .github/workflows/release-skills.yml:1-103 #38d7663299ab
  - Cargo.toml:7-10 #b7c0248bbfa6
  - pyproject.toml:5-8 #36e3866f7cbc
  - python/skrun/__init__.py:5 #4e5db8348efa
---

| workflow | trigger | does |
|---|---|---|
| `python.yml` | push / PR to `main` touching code | the `check.sh` steps, uploads the wheel |
| `publish-crates.yml` | tag `v*` or manual | `cargo test -p skrun`, dry-run, `cargo publish -p skrun` |
| `publish-python.yml` | tag `v*` or manual | `scripts/check.sh ci`, then PyPI publish (`skip-existing`) |
| `release-skills.yml` | tag `<skill>@<version>` or manual | builds, smoke-runs and attaches `<skill>@<version>-x86_64-linux.tar.gz` (`artifact.json`, `SKILL.md`, `schema`, `bin`) to a GitHub release |

`release-skills.yml` accepts only `regex-finder` and `cdp-browser`, and uses the CLI itself to build and run each example's `smoke.json`. Crates.io and PyPI publishing use trusted publishing (`id-token: write`), not stored tokens. Versions are set separately: the workspace `Cargo.toml` says 0.1.1 (the published crate), while `pyproject.toml` and `python/skrun/__init__.py` say 0.1.0 (the published PyPI package). Bump all three when releasing; `skip-existing` means an unbumped Python version is silently not re-uploaded.
