---
title: The suggested tier
order: 40
evidence:
  - crates/skrun/src/store.rs:126-146 #1c64bc867fe6
  - crates/skrun/src/store.rs:1353-1429 #4552b87bd26a
  - crates/skrun/src/store.rs:1220-1291 #7d90f2a2eed9
  - crates/skrun/src/risk.rs:43-47 #7c17cab8d000
  - crates/skrun/src/risk.rs:347-376 #1cd453710aa3
  - crates/skrun/src/risk.rs:866-872 #9b7b118c038d
---

`suggest_tier` gives each version a `tier` and `tier_reasons` from the first row that applies:

| tier | when | meant to run in |
|---|---|---|
| `L3` | any high finding | a microVM, never outside one |
| `L2` | a non-Python script (shell, JavaScript, TypeScript), a third-party import, or a risky stdlib import (`subprocess`, `socket`, `urllib`, `http`, `threading`, …) | a container |
| `L1` | a filesystem import (`os`, `pathlib`, `shutil`, `io`, …) or an `open(` call | a sandbox |
| `L0` | Python scripts that only compute | a plain process |
| `none` | no scripts | nothing to run |

A Python import is third-party unless it is in CPython 3.14's `sys.stdlib_module_names` or a local module: a `.py` file, or a folder holding one, in the script's folder or any folder above it up to the skill root. Risky modules are never local, so bundling a `subprocess.py` cannot lower the tier; it is a `shadows_stdlib` finding instead. Only Python imports are parsed; other languages go to `L2` by language.

The tier is a recorded suggestion: skrun has no sandbox yet, and nothing reads the tier back.
