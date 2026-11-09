---
id: sec44-object-field-watch
title: Variables 里对字段 Add Field Watchpoint（只监视这个对象）
features: 第 44 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec44-object-field-watch
pr:
claimed: 2026-11-09
files: src/ui/debug_view.rs, src/ui/debug_service.rs, src/index/method_breakpoint.rs
---

## 要做什么

照 IntelliJ：调试暂停时，在 Variables 里右键对象的字段 → Add Field Watchpoint，在这个字段的声明行设字段监视点，Instance filters 填上这个对象（只在这个对象的字段被写时停）。已有监视点时把对象加进它的 Instance filters。

不做：没有 backing field 的 Kotlin 属性（自定义 getter），留在 FEATURES.md 那一行。

## 进度
