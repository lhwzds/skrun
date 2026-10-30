---
title: 
source: #6dcdc2198619
---

每个操作的起点都一样：`load_artifact` 把 skill 目录读取为一个经过校验的 `SkillArtifact`。之后 `build_skill` 和 `run_skill` 按它的 `kind` 分发处理；`install_local_skill` 则复制目录，跳过 `.venv` 和构建缓存。没有守护进程、缓存或注册中心，文件系统就是唯一的状态。
