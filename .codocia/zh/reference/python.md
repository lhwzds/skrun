---
title: Python API
source: #5860e01daccb
---

| 调用 | 返回值 |
|---|---|
| `skrun.skill(id_or_path, root=None)` | `SkillHandle`（加载并校验制品） |
| `SkillHandle.call(value=None, timeout=60.0)` | skill 输出的 `dict` |
| `skrun.list_skills(root=None)` | `list[SkillArtifact]` |
| `skrun.install_local_skill(source, root=None, *, skill_id=None, overwrite=False)` | 按已安装清单中的 id 查找到的 `SkillHandle` |
| `skrun.build_skill(root, target_dir=None)` | `SkillArtifact` |
| `skrun.load_artifact(root)` | `SkillArtifact` |

`SkillRuntime(root=..., uv=...)` 把同样的操作组织在同一个 skill 根目录下。所有失败（包括缺少原生扩展）都会抛出 `SkillRuntimeError`。有两处与 CLI 不同：`timeout` 会被截断为整秒；`SkillRuntime`/`SkillHandle` 上的 `uv` 字段只是存着，并不会传给原生调用，所以 Python skill 总是运行 PATH 中找到的 `uv`。

已知问题：`install_local` 返回 `self.skill(artifact.id)`。当用 `skill_id=` 重命名一个带 `artifact.json` 的 skill 时，副本保留清单中的 id，因此 handle 解析到 `<root>/<original id>`，而不是副本。请用 `skrun.skill(skill_id, root=...)` 查找它。
