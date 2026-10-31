---
title: The suggested tier
order: 40
evidence:
  - crates/skrun/src/store.rs:155-175 #1c64bc867fe6
  - crates/skrun/src/store.rs:1672-1814 #41287db5936c
  - crates/skrun/src/risk.rs:45-49 #7c17cab8d000
  - crates/skrun/src/risk.rs:349-378 #1cd453710aa3
  - crates/skrun/src/risk.rs:869-875 #9b7b118c038d
---

`suggest_tier` records the first matching row as `tier`, with `tier_reasons`:

| tier | when | meant to run in |
|---|---|---|
| `L3` | a high finding, an `unscanned` script, or a file shadowing a risky module (`subprocess`, `socket`, `threading`, …) | a microVM |
| `L2` | a non-Python script (shell, JavaScript, TypeScript, `other`), a third-party or risky import, an `unscanned` or `unparsed` file, any `shadows_stdlib` | a container |
| `L1` | a filesystem import (`os`, `pathlib`, `shutil`, `io`, …) or an `open(` call | a sandbox |
| `L0` | Python scripts that only compute | a plain process |
| `none` | no scripts | nothing to run |

A bundled `subprocess.py` raises the tier, never lowers it. The tier is a recorded suggestion: skrun has no sandbox yet, and nothing reads it back.
