---
id: sec42-introduce-param-this
title: Introduce Parameter：表达式里的 this（含 Kotlin 扩展函数的 receiver）在调用处换成调用的接收者
features: 第 42 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-introduce-param-this
pr: 255
claimed: 2026-11-09
files: src/index/introduce_parameter.rs, src/domain/introduce_parameter.rs, src/index/inline_function.rs
---

## 要做什么

照 IntelliJ 的 Introduce Parameter：表达式里用到 `this`（Java / Kotlin 类成员里的 `this.rate`，或 Kotlin 扩展函数的 receiver `this.length`、`this`）时，`cart.total(…)`、`s.shout(…)` 这类带接收者的调用处，新实参里的 `this` 换成接收者（`cart.rate`、`s.length`）。现在 `this` 原样留下，指到调用方自己。嵌套的 lambda（Kotlin）、匿名类、局部类里的 `this` 不换。

## 进度

- 2026-11-09：做完（PR #255）。`ParameterPlan` 加 `this_refs`（表达式里所在函数自己的 `this`，相对位置）；`index::introduce_parameter::plan` 收集 Kotlin `this_expression`（只要不带标签的 `this`）和 Java `this`，跳过 lambda、匿名函数、object、类里的；`domain::introduce_parameter::argument_for` 在调用有接收者时把它们换成接收者。顺手修了接收者不是简单名字（`(a + b).shout(…)`）时成员前缀没加括号。Inline Function 的 `ParameterPlan` 里 `this_refs` 为空（内联时 `this` 的处理不在这次范围）。测试 `this_becomes_the_receiver`；Linux 实测扩展函数。
