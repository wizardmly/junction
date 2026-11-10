---
id: sec44-kotlin-property-accessors
title: 字段监视点：没有 backing field 的 Kotlin 属性停在它的 getter / setter
features: 第 44 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec44-kotlin-property-accessors
pr:
claimed: 2026-11-10
files: src/usecase/debug_session.rs
---

## 要做什么

照 IntelliJ 的 `KotlinFieldBreakpoint`：Kotlin 属性没有 backing field（只有自定义 getter，如 `val size: Int get() = items.size`）时，字段监视点改成停在属性的访问器入口：Field access 停在 getter（`getSize` / `isReady`），Field modification 停在 setter（`setSize`）。现在这种监视点找不到字段，从来不停。

## 进度
