---
id: sec35-more-type-intentions-3
title: Alt+Enter 更多需要类型的意图（第三批：lambda 转匿名类、filterIsInstance 等）
features: 第 35 节
status: claimed
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec35-more-type-intentions-3
pr:
claimed: 2026-11-08
files: src/index/lambdas.rs, src/index/intentions.rs, src/domain/intentions.rs
---

## 要做什么

接着 `sec35-more-type-intentions-2`（PR #240，作用域函数互转）。候选：

- Java：Replace lambda with anonymous class（要知道目标的函数式接口和方法签名）。
- Kotlin：Replace with 'filterIsInstance'（`filter { it is T }.map { it as T }`）、Add / remove explicit lambda parameter types；`with(x) { … }` 和 `x.run { … }` 互转。
- 先对照 IntelliJ 的意图列表（intellij-community 的 plugins/kotlin 和 java/java-impl 下的 intention / inspection）确认标题和行为，每做一批开一个 PR。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
