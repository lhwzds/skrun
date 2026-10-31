---
title: Python API
order: 60
evidence:
  - python/skrun/__init__.py:7-35 #5e1b57e9df4d
  - python/skrun/skill.py:172-234 #6e6c2ff298b5
  - python/skrun/skill.py:251-292 #48ea8efc1c09
---

| call | returns |
|---|---|
| `skrun.skill(id_or_path, root=None)` | `SkillHandle` (loads and validates the artifact) |
| `SkillHandle.call(value=None, timeout=60.0)` | the skill's output `dict` |
| `skrun.list_skills(root=None)` | `list[SkillArtifact]` |
| `skrun.install_local_skill(source, root=None, *, skill_id=None, overwrite=False)` | `SkillHandle` looked up by the installed manifest's id |
| `skrun.build_skill(root, target_dir=None)` | `SkillArtifact` |
| `skrun.load_artifact(root)` | `SkillArtifact` |

`SkillRuntime(root=..., uv=...)` groups the same operations around one skills root. Every failure, including a missing native extension, raises `SkillRuntimeError`. Two details differ from the CLI: `timeout` is truncated to whole seconds, and the `uv` field on `SkillRuntime`/`SkillHandle` is stored but not passed to the native call, so Python skills always run the `uv` found on PATH.

Known issue: `install_local` returns `self.skill(artifact.id)`. When `skill_id=` renames a skill that has `artifact.json`, the copy keeps the manifest's id, so the handle resolves `<root>/<original id>`, not the copy. Look it up with `skrun.skill(skill_id, root=...)`.
