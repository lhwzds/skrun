---
title: 为什么 catalog 的输出是确定的
source: #ef1e3b98a3ca
---

`catalog` 从不运行 skill，也不调用 git 或网络；只有 `registry --write` 和 `index --out` 会写文件。`build_index` 按 id 排序 skill、按路径排序脚本，不含时间戳或绝对路径。每个 skill 记录：frontmatter、入口文件 `bytes`、脚本（`.py`、`.sh`、`.js`/`.mjs`、`.ts`；Python 附带导入）以及相对链接（与注册表单元格同样解析）和各目标是否存在。输入相同，`skills.json` 就相同，所以 `check` 能发现过期的副本。这里不做任何遮蔽：提交前请先检查。

`reconcile_registry` 只改动人工维护的 `SKILL_REGISTRY.md` 中围栏代码块之外的 `| Skill | When to read | Entrypoint |` 表格（反引号或 `~~~` 围栏，只在开启它的容器内关闭：顶层、列表项或引用块；信息串含反引号的行是行内代码）。有效的行原样按序保留；入口缺失、不是 skill 或重复的行被删除；未登记的 skill 追加 `| name | summary | [id](entrypoint) |`（描述的第一句，≤160 个字符）。单元格支持 `\|` 和 `\\`；含括号、空格、`<` 或 `>` 的路径写成 `[id](<path>)`。分隔行、其他行和换行符保持不变；没有表格时报错且不写入。已知缺陷：有序列表中以 4 个以上空格缩进关闭的围栏不会关闭（`registry` 随后找不到表格），列表/引用嵌套顺序颠倒可能提前关闭围栏，含 `>` 的路径无法往返。
