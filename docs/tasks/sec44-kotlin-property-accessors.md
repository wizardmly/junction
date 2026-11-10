---
id: sec44-kotlin-property-accessors
title: 字段监视点：没有 backing field 的 Kotlin 属性停在它的 getter / setter
features: 第 44 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec44-kotlin-property-accessors
pr: https://github.com/wizardmly/junction/pull/321
claimed: 2026-11-10
files: src/usecase/debug_session.rs
---

## 要做什么

照 IntelliJ 的 `KotlinFieldBreakpoint`：Kotlin 属性没有 backing field（只有自定义 getter，如 `val size: Int get() = items.size`）时，字段监视点改成停在属性的访问器入口：Field access 停在 getter（`getSize` / `isReady`），Field modification 停在 setter（`setSize`）。现在这种监视点找不到字段，从来不停。

## 进度

- 2026-11-10：做完（PR #321）。`usecase::debug_session::DebugSession::place` 里字段监视点的 `field_id` 找不到字段、且断点在 `.kt` 文件时，按 `accessor_names`（`getX` / `setX`，`isX` 属性的 getter 是它自己、setter 是 `setX`）找类里的访问器，Field access 在 getter、Field modification 在 setter 的第一条代码上设普通断点；这些请求记在 `DebugSession.accessors`，删断点时按断点清除（不是按字段监视）。命中按普通断点处理，日志是 “Breakpoint reached at …”。测试 `property_watchpoints_without_a_field_stop_in_the_accessors`；Linux 实测见 FEATURES.md 第 44 节。
- 没做：热替换后不重新设置访问器上的断点（字段监视点在 hot swap 里本来就跳过）。
