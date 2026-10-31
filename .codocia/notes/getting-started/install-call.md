---
title: Install it and call it from Python
order: 30
evidence:
  - crates/skrun/src/main.rs:89-114 #2ccbea2616a2
  - python/skrun/skill.py:236-240 #ad1c8847cbc7
---

`install-local` copies a skill directory into the skills root (`~/.skrun/skills` by default; see [[reference]]):

```bash
skrun skill install-local /tmp/rust-echo
skrun skill list
skrun skill run --id rust-echo --input '{"ok":true}'
```

```python
import skrun

skrun.skill("rust-echo").call({"ok": True})        # by id, from the skills root
skrun.skill("/tmp/rust-echo").call({"ok": True})   # or by path
```

`skrun.skill` treats its argument as a path when that directory contains `artifact.json` or `SKILL.md`, otherwise as an id under the skills root. Because install copies, re-install with `--overwrite` after rebuilding a skill ([[how-it-works]]).
