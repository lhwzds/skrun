---
title: What counts as a secret
order: 35
evidence:
  - crates/skrun/src/catalog.rs:2608-2683 #d72201d6b9dd
  - crates/skrun/src/catalog.rs:2685-2763 #5c0c576a42b8
  - crates/skrun/src/catalog.rs:2786-2890 #0ca90eb3405c
  - crates/skrun/src/catalog.rs:2892-2928 #5ed281c03b65
  - commit:ae10cbf
---

Beyond the tokens of [[reference]], `line_has_secret` flags an assignment (`=` or `:`, quoted keys too) whose key has `password`, `passwd`, `pwd`, `secret`, `token`, `api_key`, `client_secret`, access or private key as a whole word (`x-api-key`, or last segment as in `POSTGRES_PASSWORD`; not `max_tokens`) and whose value looks real: six or more non-space characters, not a `$` or `%` variable, expression, type annotation or placeholder (`<token>`, `changeme`, `YOUR_KEY`, …). Findings give `path:line` only; `redact` prints such strings as `<redacted>`. Known gap: a YAML value starting with a placeholder word, then a real secret.
