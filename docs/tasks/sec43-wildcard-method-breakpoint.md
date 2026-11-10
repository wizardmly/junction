---
id: sec43-wildcard-method-breakpoint
title: 按类名 + 方法名的通配方法断点（Breakpoints 对话框 + › Java Method Breakpoints）
features: 第 43 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec43-wildcard-method-breakpoint
pr:
claimed: 2026-11-10
files: src/usecase/debug_session.rs, src/infra/breakpoint_store.rs, src/ui/debug_service.rs, src/ui/breakpoints_dialog.rs
---

## 要做什么

照 IntelliJ 的 `WildcardMethodBreakpoint`：Breakpoints 对话框的 + 里选 Java Method Breakpoints，填类名模式（`*` 通配，如 `com.example.*Service`）和方法名，不需要源码行；调试时在所有匹配的类（包括库里的类）里同名方法的第一条代码上停。条件、日志、Method entry / exit 等和普通方法断点一样。

不做：lambda 的实现（留在 FEATURES.md 那一行）。

## 进度
