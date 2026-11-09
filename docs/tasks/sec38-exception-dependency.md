---
id: sec38-exception-dependency
title: 异常断点参与依赖（Disable until hitting the following breakpoint）
features: 第 38 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec38-exception-dependency
pr:
claimed: 2026-11-09
files: src/usecase/debug_session.rs, src/infra/breakpoint_store.rs, src/ui/debug_service.rs, src/ui/breakpoints_dialog.rs
---

## 要做什么

照 IntelliJ 的 Breakpoints 对话框：异常断点也有 “Disable until hitting the following breakpoint” 和 After hit，下拉框里可以选行断点或异常断点；行断点的下拉框里也能选异常断点作为 master。异常断点命中（过滤和条件都通过）后，依赖它的断点生效。

不做：异常断点按文件分组（留在 FEATURES.md 那一行）。

## 进度
