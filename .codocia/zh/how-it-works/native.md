---
title: Python 边界
source: #e68caa7cf40d
---

`skrun-native` 暴露五个函数（`runtime_load_artifact_json`、`runtime_list_skills_json`、`runtime_build_skill_json`、`runtime_run_skill_json`、`runtime_install_local_skill_json`）。每个函数都接收字符串并返回 JSON 字符串，所以 ABI 只是 serde JSON，Python 的 dataclass 与 Rust 结构体一一对应。每次调用都在 `py.detach` 中运行，构建或 skill 进程运行期间会释放 GIL，其他 Python 线程可以继续工作。Rust 错误会变成 `RuntimeError`，SDK 再把它们重新包装为 `SkillRuntimeError`。对 list 和 install，`root` 为空或缺失时回退到 `default_skills_dir`；load、build 和 run 必须显式传入 `root`。从 Python 发起的构建总是使用默认的 Cargo/uv 命令和 release profile，只有 `target_dir` 可配置。
