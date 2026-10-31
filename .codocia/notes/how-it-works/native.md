---
title: The Python boundary
order: 60
evidence:
  - crates/py/src/lib.rs:39-151 #350319ae53da
  - python/skrun/skill.py:316-331 #faab132ec883
---

`skrun-native` exposes five functions (`runtime_load_artifact_json`, `runtime_list_skills_json`, `runtime_build_skill_json`, `runtime_run_skill_json`, `runtime_install_local_skill_json`). Each takes strings and returns a JSON string that the Python dataclasses mirror. Each call runs inside `py.detach`, which releases the GIL while a build or skill process runs. Rust errors become `RuntimeError` and the SDK rewraps them as `SkillRuntimeError`. For list and install, an empty or missing `root` falls back to `default_skills_dir`; load, build and run need an explicit `root`. Builds from Python always use the default Cargo/uv commands and release profile; only `target_dir` is configurable.
