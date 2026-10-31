---
title: 工作原理
---

# 工作原理

<sub>[English](../04-how-it-works.md) · **中文**</sub>

每个操作的起点都一样：`load_artifact` 把 skill 目录读取为一个经过校验的 `SkillArtifact`。之后 `build_skill` 和 `run_skill` 按它的 `kind` 分发处理；`install_local_skill` 则复制目录，跳过 `.venv` 和构建缓存。没有守护进程、缓存或注册中心，文件系统就是唯一的状态。

<sub>来源: [runtime.rs:594-609](../../crates/skrun/src/runtime.rs#L594-L609)</sub>

## 加载 skill 目录

如果存在 `artifact.json`，先解码它，再由 `apply_skill_markdown` 用 `SKILL.md` 填充清单中留空的元数据字段（`description`、`tags`、`suggested_tools`、`content`、`source_ref`）。清单始终优先。如果没有 `artifact.json`，目录中必须有 `SKILL.md`，并成为一个以目录名命名的 `markdown` skill。无论哪种情况，`validate` 都在最后运行，所以每个调用方（CLI、Rust 库、Python）遇到错误清单时看到的错误都一样。

<sub>来源: [runtime.rs:594-609](../../crates/skrun/src/runtime.rs#L594-L609) · [runtime.rs:427-447](../../crates/skrun/src/runtime.rs#L427-L447) · [runtime.rs:392-425](../../crates/skrun/src/runtime.rs#L392-L425)</sub>

## 一次调用如何执行

```mermaid
sequenceDiagram
  participant C as caller
  participant R as run_skill
  participant P as skill process
  C->>R: dir, JSON value, RunOptions
  R->>R: canonicalize dir, load_artifact
  R->>P: spawn in dir (rust: entry binary / python: uv run --project dir python entry)
  R->>P: stdin = JSON + "\n", then close
  loop every 10 ms until exit or timeout
    R->>P: try_wait
  end
  P-->>R: exit status, stdout, stderr
  R->>R: non-zero exit -> error; stdout must parse as a JSON object
  R-->>C: SkillRunOutput { value, stderr, exit_code }
```

工作目录就是 skill 目录，所以 skill 可以用相对路径打开自己的文件。写入一个值后就关闭 stdin，skill 因此只需一直读到 EOF。stderr 会被捕获并作为诊断信息返回，从不解析。

<sub>来源: [runtime.rs:675-717](../../crates/skrun/src/runtime.rs#L675-L717) · [runtime.rs:1006-1069](../../crates/skrun/src/runtime.rs#L1006-L1069)</sub>

## 哪些情况算失败

- **超时**：超过 `timeout`（默认 60 秒）后进程会被杀掉，调用以 `skill process timed out after Ns` 失败。
- **非零退出**：调用失败，并带上退出码和去掉首尾空白的 stderr。stdout 会被丢弃，所以如果 skill 打印结构化错误内容后以 1 退出（`regex-finder` 就是这样），这些内容会丢失；请同时把消息写到 stderr，或者以 0 退出并返回 `"ok": false`。
- **输出不合法**：stdout 必须恰好是一个 JSON 值，而且必须是对象；数组、标量或多个 JSON 值都会报错（一个对象分成多行写是可以的）。

stdout 和 stderr 只在进程退出后才收集。以下未经测试，仅根据代码推断：skill 如果在退出前写入的数据超过操作系统管道缓冲区，会一直阻塞到超时；请把大结果写到文件里再返回文件路径。

<sub>来源: [runtime.rs:1027-1069](../../crates/skrun/src/runtime.rs#L1027-L1069) · [main.rs:27-35](../../examples/skills/regex-finder/src/main.rs#L27-L35)</sub>

## 构建

`rust_binary`：运行 `cargo build --manifest-path <dir>/Cargo.toml --target-dir <target>`，profile 为 `release` 时加 `--release`，否则加 `--profile <name>`，然后把 `<target>/<dir>/<id>`（加上平台的可执行文件后缀）复制到 `entry`。其中 `<dir>` 是 Cargo 的输出目录：`dev` 和 `test` 为 `debug`，`release` 和 `bench` 为 `release`，其他情况为 profile 名。因此二进制名必须与 skill id 相同。无论 profile 是什么都会复制到 `entry`，所以使用脚手架生成的 entry 时，debug 构建的产物也会落在 `bin/release/...`。

`python_uv`：仅在缺少 `uv.lock` 时运行 `uv lock --project <dir>`，然后运行 `uv sync --project <dir> --locked`。锁文件是 skill 的一部分，因此安装可复现。

两种构建步骤都会捕获输出，失败时只报告 stderr。`markdown` skill 无法构建。

<sub>来源: [runtime.rs:661-673](../../crates/skrun/src/runtime.rs#L661-L673) · [runtime.rs:946-992](../../crates/skrun/src/runtime.rs#L946-L992) · [runtime.rs:1071-1083](../../crates/skrun/src/runtime.rs#L1071-L1083)</sub>

## 安装与列出

`install_local_skill` 先校验源目录，再把它递归复制到 `<root>/<id>`。目标已存在时会以 `installed skill already exists` 失败，除非设置了 `overwrite`；设置 `overwrite` 时会先删除目标，而把目录安装到它自身时会直接返回，不做复制。复制完成后会重新加载并返回副本；只校验元数据，所以没构建过的 Rust skill 也能装上，要到运行时才失败。构建好的二进制会被复制，但任意层级下名为 `.venv`、`__pycache__`、`target` 或 `.git` 的目录都会跳过，除非 artifact 的 `entry` 就在其中。用 `--id` 重命名只改变目录名；复制过去的 `artifact.json` 仍保留原来的 `id`。

`list_installed_skills` 扫描根目录下直接包含 `artifact.json` 或 `SKILL.md` 的子目录，跳过加载失败的目录，并按 id 排序。根目录不存在时返回空列表。

<sub>来源: [runtime.rs:727-787](../../crates/skrun/src/runtime.rs#L727-L787) · [runtime.rs:1096-1113](../../crates/skrun/src/runtime.rs#L1096-L1113) · [runtime.rs:760-766](../../crates/skrun/src/runtime.rs#L760-L766)</sub>

## Python 边界

`skrun-native` 暴露五个函数（`runtime_load_artifact_json`、`runtime_list_skills_json`、`runtime_build_skill_json`、`runtime_run_skill_json`、`runtime_install_local_skill_json`）。每个函数都接收字符串并返回 JSON 字符串，Python 的 dataclass 与之一一对应。每次调用都在 `py.detach` 中运行，构建或 skill 进程运行期间会释放 GIL。Rust 错误会变成 `RuntimeError`，SDK 再把它们重新包装为 `SkillRuntimeError`。对 list 和 install，`root` 为空或缺失时回退到 `default_skills_dir`；load、build 和 run 必须显式传入 `root`。从 Python 发起的构建总是使用默认的 Cargo/uv 命令和 release profile，只有 `target_dir` 可配置。

<sub>来源: [lib.rs:39-151](../../crates/py/src/lib.rs#L39-L151) · [skill.py:316-331](../../python/skrun/skill.py#L316-L331)</sub>

## 为什么 catalog 的输出是确定的

`catalog` 从不运行 skill，也不调用 git 或网络；只有 `registry --write` 和 `index --out` 会写文件。`build_index` 按 id 排序 skill、按路径排序脚本，不含时间戳或绝对路径。每个 skill 记录：frontmatter、入口文件 `bytes`、脚本（`.py`、`.sh`、`.js`/`.mjs`、`.ts`；Python 附带导入）以及相对链接（与注册表单元格同样解析）和各目标是否存在。输入相同，`skills.json` 就相同，所以 `check` 能发现过期的副本。这里不做任何遮蔽：提交前请先检查。

`reconcile_registry` 只改动人工维护的 `SKILL_REGISTRY.md` 中围栏代码块之外的 `| Skill | When to read | Entrypoint |` 表格（反引号或 `~~~` 围栏，只在开启它的容器内关闭：顶层、列表项或引用块；信息串含反引号的行是行内代码）。有效的行原样按序保留；入口缺失、不是 skill 或重复的行被删除；未登记的 skill 追加 `| name | summary | [id](entrypoint) |`（描述的第一句，≤160 个字符）。单元格支持 `\|` 和 `\\`；含括号、空格、`<` 或 `>` 的路径写成 `[id](<path>)`。分隔行、其他行和换行符保持不变；没有表格时报错且不写入。已知缺陷：有序列表中以 4 个以上空格缩进关闭的围栏不会关闭（`registry` 随后找不到表格），列表/引用嵌套顺序颠倒可能提前关闭围栏，含 `>` 的路径无法往返。

<sub>来源: [catalog.rs:14-18](../../crates/skrun/src/catalog.rs#L14-L18) · [catalog.rs:420-452](../../crates/skrun/src/catalog.rs#L420-L452) · [catalog.rs:481-585](../../crates/skrun/src/catalog.rs#L481-L585) · [catalog.rs:2039-2082](../../crates/skrun/src/catalog.rs#L2039-L2082) · [catalog.rs:1332-1365](../../crates/skrun/src/catalog.rs#L1332-L1365) · +6</sub>

## 搜索如何排序

`search_skills` 把查询按空白拆成词并转为小写，再按子串匹配，所以中文无需分词。每个词都必须命中某个字段（AND），并累加它命中的每个字段的权重：id 5，frontmatter name 4，注册表标题 4，注册表摘要、description 和每个 tag 3，入口正文 1。结果按分数排序，同分按 id；`--has-scripts` 会去掉所有 flat skill。结果显示注册表标题（否则为 name）和注册表摘要（否则为 description 的第一句：截至 `. `、`? `、`! `、`。`、`？`、`！` 或换行，最多 120 个字符），截断前先做遮蔽，因此密钥绝不会被部分打印。

<sub>来源: [catalog.rs:714-738](../../crates/skrun/src/catalog.rs#L714-L738) · [catalog.rs:785-861](../../crates/skrun/src/catalog.rs#L785-L861) · [catalog.rs:901-955](../../crates/skrun/src/catalog.rs#L901-L955) · [catalog.rs:1013-1051](../../crates/skrun/src/catalog.rs#L1013-L1051) · [catalog.rs:75-87](../../crates/skrun/src/catalog.rs#L75-L87)</sub>
