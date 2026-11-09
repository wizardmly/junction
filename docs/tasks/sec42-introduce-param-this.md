---
id: sec42-introduce-param-this
title: Introduce Parameter：表达式里的 this（含 Kotlin 扩展函数的 receiver）在调用处换成调用的接收者
features: 第 42 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-introduce-param-this
pr:
claimed: 2026-11-09
files: src/index/introduce_parameter.rs, src/domain/introduce_parameter.rs, src/index/inline_function.rs
---

## 要做什么

照 IntelliJ 的 Introduce Parameter：表达式里用到 `this`（Java / Kotlin 类成员里的 `this.rate`，或 Kotlin 扩展函数的 receiver `this.length`、`this`）时，`cart.total(…)`、`s.shout(…)` 这类带接收者的调用处，新实参里的 `this` 换成接收者（`cart.rate`、`s.length`）。现在 `this` 原样留下，指到调用方自己。嵌套的 lambda（Kotlin）、匿名类、局部类里的 `this` 不换。

## 进度
