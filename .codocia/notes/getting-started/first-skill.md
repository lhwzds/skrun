---
title: Create, build and run a skill
order: 20
evidence:
  - crates/skrun/src/runtime.rs:623-659 #06faaaffb8b7
  - crates/skrun/src/runtime.rs:828-866 #8f42fc033a58
  - crates/skrun/src/runtime.rs:954-992 #caed552138d3
  - crates/skrun/src/main.rs:123-131 #d689e059c180
---

```bash
skrun skill new --kind rust_binary --id rust-echo /tmp/rust-echo
skrun skill build /tmp/rust-echo
skrun skill run --input '{"ok":true}' /tmp/rust-echo
# {
#   "ok": true
# }
```

`new` writes a standalone Cargo project (`Cargo.toml` with an empty `[workspace]`, `src/main.rs` that echoes stdin), permissive `schema/input.json` and `schema/output.json`, a `SKILL.md`, and `artifact.json` with `entry: bin/release/rust-echo`. `build` runs Cargo and copies the binary to that `entry` path. `run` pipes the input to the binary and pretty-prints the JSON object it returns.

For Python, use `--kind python_uv`: the scaffold is a `pyproject.toml` plus an echoing `skill.py`, and `build` runs `uv lock`/`uv sync`.
