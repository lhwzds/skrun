---
title: 冒烟测试验证了什么
source: #7be03da597ba
---

`package.py smoke` 在 `/tmp` 下运行，因此导入的是已安装的包而不是源码树。它会写一个临时的 `rust_binary` skill，其 entry 是一个运行 `cat` 的 `#!/bin/sh` 脚本，然后断言 `skrun.skill(root).call({'native': True})` 返回相同的 dict。用 shell 脚本就够了，因为 `rust_binary` 只要求 `entry` 处有一个可执行文件；这也让它成为在测试中包装任意现有可执行文件的快捷方法。
