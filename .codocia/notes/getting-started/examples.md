---
title: Try a real example
order: 40
evidence:
  - examples/skills/regex-finder/artifact.json #feb72047d29b
  - examples/skills/regex-finder/smoke.json #c451569233dd
  - examples/skills/regex-finder/src/main.rs:91-117 #81b494aa100d
  - .github/workflows/release-skills.yml:75-82 #8d82618c2057
---

`examples/skills` holds `rust-echo`, `python-echo`, `regex-finder` (Rust `regex` crate) and `cdp-browser` (Chrome DevTools Protocol control). The two larger ones show how an existing command-line tool becomes a skill by adding `artifact.json` and JSON schemas, and each ships a `smoke.json` input:

```bash
skrun skill build examples/skills/regex-finder
skrun skill run --input "$(cat examples/skills/regex-finder/smoke.json)" examples/skills/regex-finder
```

`regex-finder` expects `{"action": "match", "input": {"pattern": ..., "text": ...}}` (or `"action": "describe"`) and answers with an `ok`/`action`/`data`/`error` envelope. Building writes `bin/release/regex-finder` inside the example directory.
