---
id: sec35-type-intentions
title: Alt+Enter 其他需要类型的意图
features: 第 35 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec35-type-intentions
pr: 235
claimed: 2026-11-08
files: src/domain/intentions.rs, src/index/intentions.rs
---

## 要做什么

docs/FEATURES.md 第 35 节里“其他需要类型的意图”这一行。先对照 IntelliJ 的 Kotlin / Java 意图列表（intellij-community 的 plugins/kotlin/idea/.../intentions、java/java-impl/.../intention），挑日常常用、需要类型信息才能判断的，逐个做；每做一批开一个 PR。

## 进度

- 2026-11-08（PR #235）：做了三个意图，都在新文件 `src/index/lambdas.rs`，在 `src/ui/file_editor/fixes.rs` 的 Java / Kotlin 类型意图列表里接上，`src/domain/intentions.rs` 的 `FAMILIES` 加了三项（Settings › Editor › Intentions 可关）。
  - `anonymous_to_lambda`（Java Replace with lambda）：`functional()` 先用 `Classes` 找接口声明数抽象方法，找不到再查 `FUNCTIONAL` 常量表。
  - `lambda_to_method_reference`（Java Replace lambda with method reference）：不需要类型，参数写了类型时才做 `Type::method`。
  - `lambda_to_reference`（Kotlin Convert lambda to reference）：`it` 的类型从调用的接收者推断（`introduce::kotlin_type`，再退到 `types::type_of`），只认 `ELEMENT_FUNCTIONS` 里的函数。
  - Xvfb 实测三项都正常；单测在 `lambdas.rs` 里。
- 剩下的拆到新任务 `sec35-more-type-intentions`。
