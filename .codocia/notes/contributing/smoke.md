---
title: What the smoke test proves
order: 20
evidence:
  - python/scripts/package.py:64-88 #8a4ea9d7a2f3
---

`package.py smoke` runs from `/tmp`, so it imports the installed package rather than the source tree. It writes a throwaway `rust_binary` skill whose entry is a `#!/bin/sh` script running `cat`, then asserts that `skrun.skill(root).call({'native': True})` returns the same dict. A shell script is enough because `rust_binary` only requires an executable file at `entry`, which also makes this a quick way to wrap any existing executable for testing.
