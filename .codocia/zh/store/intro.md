---
title: 
source: #682861226e5f
---

`skrun catalog add` 把来自别处的 skill（另一个仓库、下载的 zip）复制进本地存储，`inspect` 则显示收到了什么。两者都不会执行、构建或导入 skill 中的任何东西，也不访问网络：存储是在决定安装之前查看 skill 的地方（[[how-it-works]]）。

```bash
skrun catalog add <dir|zip> [--store <dir>] [--id <id>] [--from <provenance>] [--json]
skrun catalog inspect <id> [--store <dir>] [--json]
skrun catalog inspect --list [--store <dir>] [--json]
```

`add` 打印 `stored`、`already stored` 或 `already stored (record refreshed)`，后面跟 id、版本哈希、文件数和发现项数量以及等级；`--json` 打印该版本的记录。`inspect` 打印每个版本的来源、脚本、按严重程度分组的发现项和等级理由；`--list` 为每个 id 的最新版本打印一行（`--json` 在这里同样可用，只是用法说明里没写）。查看一个从未添加过的 id 会失败。打印的每个字符串都会经过 `redact`，手工编辑过的记录也一样。
