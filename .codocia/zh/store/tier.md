---
title: 建议的等级
source: #139b3f463900
---

`suggest_tier` 按第一条适用的行为每个版本给出 `tier` 和 `tier_reasons`：

| 等级 | 条件 | 适合运行在 |
|---|---|---|
| `L3` | 任意 high 发现项 | microVM，绝不在其外 |
| `L2` | 非 Python 脚本（shell、JavaScript、TypeScript）、第三方导入，或危险的标准库导入（`subprocess`、`socket`、`urllib`、`http`、`threading` 等） | 容器 |
| `L1` | 文件系统导入（`os`、`pathlib`、`shutil`、`io` 等）或 `open(` 调用 | 沙箱 |
| `L0` | 只做计算的 Python 脚本 | 普通进程 |
| `none` | 没有脚本 | 无需运行 |

一个 Python 导入只要不在 CPython 3.14 的 `sys.stdlib_module_names` 中、也不是本地模块，就算第三方导入。本地模块指脚本所在文件夹或其上直到 skill 根目录的任一文件夹中的 `.py` 文件，或含有此类文件的文件夹。危险模块从不算本地模块，所以附带一个 `subprocess.py` 不能降低等级，而是记为 `shadows_stdlib` 发现项。只解析 Python 导入；其他语言按语言直接归为 `L2`。

等级只是记录下来的建议：skrun 目前没有沙箱，也没有任何地方读回这个等级。
