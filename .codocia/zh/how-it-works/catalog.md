---
title: 为什么 catalog 的输出是确定的
source: #d05ab52f4449
---

`catalog` 从不运行 skill，不调用 git，也不访问网络；只有 `registry --write` 和 `index --out` 会写文件。`build_index` 按 id 排序 skill、按路径排序脚本，不记录时间戳或绝对路径。每个 skill 记录：frontmatter（`name`、`description`、`version`、`tags`、`requires`）、入口文件 `bytes`、脚本（`.py`、`.sh`、`.js`/`.mjs`、`.ts`；Python 脚本附带顶层导入）以及相对链接和各目标是否存在。输入不变时 `skills.json` 也不变，`check` 据此可以发现过期的副本。它是数据，从不遮蔽：frontmatter 原样复制，路径也保留，提交前请检查这两者。

`reconcile_registry` 把 `SKILL_REGISTRY.md` 当作人工维护的文本。它在围栏代码块（反引号或 `~~~` 围栏）之外查找 `| Skill | When to read | Entrypoint |` 表格，所以围栏里的示例表格永远不会被改写。它原样按顺序保留有效的行，删除入口缺失、不是 skill 或重复的行，并为未登记的 skill 追加 `| name | summary | [id](entrypoint) |`，summary 取描述的第一句（最多 160 个字符）。单元格支持 `\|` 和双反斜杠转义，生成的单元格也这样转义，所以行可以原样往返；带对齐冒号的分隔行（`:---:`）保持不变。其他行和换行符保持不变；没有表格时直接报错，不写入文件。
