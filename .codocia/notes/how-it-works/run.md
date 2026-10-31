---
title: How a call runs
order: 20
evidence:
  - crates/skrun/src/runtime.rs:675-717 #279ff9e9d83d
  - crates/skrun/src/runtime.rs:1006-1069 #2cb35e53f1bc
---

```mermaid
sequenceDiagram
  participant C as caller
  participant R as run_skill
  participant P as skill process
  C->>R: dir, JSON value, RunOptions
  R->>R: canonicalize dir, load_artifact
  R->>P: spawn in dir (rust: entry binary / python: uv run --project dir python entry)
  R->>P: stdin = JSON + "\n", then close
  loop every 10 ms until exit or timeout
    R->>P: try_wait
  end
  P-->>R: exit status, stdout, stderr
  R->>R: non-zero exit -> error; stdout must parse as a JSON object
  R-->>C: SkillRunOutput { value, stderr, exit_code }
```

The working directory is the skill directory, so a skill can open its own files by relative path. Closing stdin after one value is what lets a skill simply read to EOF. Stderr is captured and returned as diagnostics, never parsed.
