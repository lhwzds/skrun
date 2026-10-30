---
title: 
order: 0
evidence:
  - crates/skrun/src/runtime.rs:594-609 #8693b9f54dd4
---

Every operation starts the same way: `load_artifact` reads the skill directory into a validated `SkillArtifact`. `build_skill` and `run_skill` then dispatch on its `kind`; `install_local_skill` copies the directory, skipping `.venv` and build caches. There is no daemon, cache or registry; the filesystem is the only state.
