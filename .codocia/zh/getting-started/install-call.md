---
title: 安装 skill 并从 Python 调用
source: #8b43685662b9
---

`install-local` 会把 skill 目录复制到 skill 根目录（默认是 `~/.skrun/skills`，见 [[reference]]）：

```bash
skrun skill install-local /tmp/rust-echo
skrun skill list
skrun skill run --id rust-echo --input '{"ok":true}'
```

```python
import skrun

skrun.skill("rust-echo").call({"ok": True})        # 按 id，从 skill 根目录查找
skrun.skill("/tmp/rust-echo").call({"ok": True})   # 或者按路径
```

如果参数对应的目录中包含 `artifact.json` 或 `SKILL.md`，`skrun.skill` 就把它当作路径，否则当作 skill 根目录下的 id。由于安装是复制，重新构建 skill 后要用 `--overwrite` 重新安装（[[how-it-works]]）。
