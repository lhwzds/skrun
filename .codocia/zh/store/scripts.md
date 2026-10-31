---
title: 如何识别脚本和导入
source: #a996a42fccea
---

文件按扩展名（`.py`、`.sh`、`.js`/`.mjs`、`.ts`）认定为脚本，否则看 `#!` 行：`sh`、`bash`、`zsh`、`dash` 是 shell，`python*` 是 Python，`node`、`deno`、`bun` 是 JavaScript，其他一律为 `other`。Python 导入只在字符串和注释之外计算，同一行 `;` 或 `:` 之后的也算（`if True: import x`），另外还有以字符串字面量调用的 `__import__` 和 `importlib.import_module`。无法分词的 Python（括号不配对、字符串未结束、缩进错误）记为 `unparsed`。已知缺陷：`__import__` 中用三引号字符串写的模块名会漏报。

一个导入只要不在 CPython 3.14 的 `sys.stdlib_module_names` 中、也不是本地模块，就算第三方导入。本地模块指脚本所在文件夹或其上层中的 `.py` 文件，或含有此类文件的文件夹。
