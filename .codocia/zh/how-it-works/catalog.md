---
title: 为什么 catalog 的输出可以放心提交
source: #df6ab3f3982a
---

`catalog` 从不运行 skill，不调用 git，也不访问网络；它只读取目录，所以输出可以提交并以 diff 审阅。`build_index` 按 id 排序 skill、按路径排序脚本，不记录时间戳或绝对路径。每个 skill 保存 frontmatter（`name`、`description`、`version`、`tags`、`requires`）、入口文件大小 `bytes`、文件夹内的 `.py`、`.sh`、`.js`/`.mjs` 和 `.ts` 脚本（Python 脚本附带导入的顶层模块），以及相对链接和目标是否存在。因此目录不变时 `skills.json` 也不变，`check` 正是据此判断提交的副本是否过期。

`reconcile_registry` 把 `SKILL_REGISTRY.md` 当作人工维护的文本。它找到 `| Skill | When to read | Entrypoint |` 表格，原样按顺序保留仍有效的行，删除入口缺失、不是 skill 或重复的行，并为未登记的 skill 追加 `| name | summary | [id](entrypoint) |`，其中 summary 是描述的第一句，截断到 160 个字符。表格之外的行和换行符保持不变。表头匹配忽略首尾空白；没有表格时直接报错，从不写入文件。
