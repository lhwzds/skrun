---
title: 
source: #66ab120d2dfb
---

`skrun catalog add` 把来自别处的 skill（另一个仓库、下载的 zip）复制进本地存储；`inspect` 显示收到了什么。两者都不会执行、构建或导入 skill 中的任何东西，也不访问网络：存储是安装之前查看 skill 的地方（[[how-it-works]]）。

```bash
skrun catalog add <dir|zip> [--store <dir>] [--id <id>] [--from <provenance>] [--json]
skrun catalog inspect <id> [--store <dir>] [--json]
skrun catalog inspect --list [--store <dir>] [--json]
```

`add` 打印 `stored`、`already stored` 或 `already stored (record refreshed)`，并附上 id、版本哈希、文件数和发现项数以及等级；`--json` 打印版本记录。`inspect` 打印每个版本的来源、脚本、按严重程度分组的发现项和等级理由；`--list` 为每个 id 用一行打印其最新版本。所有打印的字符串都经过 `redact`，手工编辑过的记录也不例外。
