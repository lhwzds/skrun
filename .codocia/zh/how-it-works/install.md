---
title: 安装与列出
source: #e3b2647dc0e8
---

`install_local_skill` 先校验源目录，再把它递归复制到 `<root>/<id>`。目标已存在时会以 `installed skill already exists` 失败，除非设置了 `overwrite`；设置 `overwrite` 时会先删除目标，而把目录安装到它自身时会直接返回，不做复制。复制完成后会重新加载并返回副本；只校验元数据，所以没构建过的 Rust skill 也能装上，要到运行时才失败。构建好的二进制会被复制，但任意层级下名为 `.venv`、`__pycache__`、`target` 或 `.git` 的目录都会跳过，除非 artifact 的 `entry` 就在其中。用 `--id` 重命名只改变目录名；复制过去的 `artifact.json` 仍保留原来的 `id`。

`list_installed_skills` 扫描根目录下直接包含 `artifact.json` 或 `SKILL.md` 的子目录，跳过加载失败的目录，并按 id 排序。根目录不存在时返回空列表。
