---
title: Adding a skill kind or command
order: 40
evidence:
  - crates/skrun/src/runtime.rs:55-61 #8a978ae89334
  - crates/skrun/src/runtime.rs:586-592 #ecdc2f73f500
  - crates/skrun/src/main.rs:376-405 #e9ba28072c5e
  - crates/skrun/src/runtime.rs:3-17 #5f4eeb70be91
---

A new kind touches every `match` on `ArtifactKind` in `runtime.rs`: `validate`, `scaffold_skill`, `build_skill`, `run_skill` and `kind_label`, plus `parse_kind` and the duplicate `kind_label` in `main.rs`. The Python side treats `kind` as a plain string and needs no change unless the kind needs new options. Keep the runtime contract intact: anything that executes must still take one JSON value on stdin and return one JSON object on stdout.

The runtime, CLI, binding and SDK modules open with a doc header (`Owns`, `Must Not`, `Inputs`, `Outputs`, `Depends On`, `Verify`). Update it when you move responsibility, and respect the `Must Not` lists: the runtime does not plan, parse `@skill` mentions, render UI or hold durable state. New operations belong in `runtime.rs` first and are then exposed through `main.rs` and `crates/py/src/lib.rs`, never implemented in Python.
