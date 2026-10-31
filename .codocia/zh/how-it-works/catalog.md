---
title: 为什么 catalog 的输出是确定的
source: #e8038c461337
---

`catalog` 从不运行 skill，不调用 git，也不访问网络；只有 `registry --write` 和 `index --out` 会写文件。`build_index` 按 id 排序 skill、按路径排序脚本，不记录时间戳或绝对路径。每个 skill 记录：frontmatter（`name`、`description`、`version`、`tags`、`requires`）、入口文件 `bytes`、脚本（`.py`、`.sh`、`.js`/`.mjs`、`.ts`；Python 脚本附带顶层导入）以及相对链接和各目标是否存在。输入相同，`skills.json` 就相同，所以 `check` 能发现过期的副本。这里不做任何遮蔽：提交前请检查 frontmatter 和路径。

`reconcile_registry` 把 `SKILL_REGISTRY.md` 当作人工维护的文本，只改动围栏代码块（反引号或 `~~~`，列表项和引用块中的也算；信息串中含反引号的行是行内代码）之外的 `| Skill | When to read | Entrypoint |` 表格。有效的行原样按顺序保留；入口缺失、不是 skill 或重复的行被删除；未登记的 skill 追加 `| name | summary | [id](entrypoint) |`，summary 取描述的第一句（≤160 个字符）。单元格支持 `\|` 和 `\\` 转义；含括号、空格、`<` 或 `>` 的路径写成 `[id](<path>)`；两种写法都能解析，所以行可以原样往返（含 `>` 的路径暂不支持）。对齐分隔行、其他行和换行符保持不变；没有表格时直接报错，不写入文件。
