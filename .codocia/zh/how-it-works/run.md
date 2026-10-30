---
title: 一次调用如何执行
source: #ed0590c11bb7
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

工作目录就是 skill 目录，所以 skill 可以用相对路径打开自己的文件。写入一个值后就关闭 stdin，skill 因此只需一直读到 EOF。stderr 会被捕获并作为诊断信息返回，从不解析。
