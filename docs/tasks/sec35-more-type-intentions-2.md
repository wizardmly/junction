---
id: sec35-more-type-intentions-2
title: Alt+Enter 更多需要类型的意图（第二批：lambda 转匿名类、作用域函数等）
features: 第 35 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec35-more-type-intentions-2
pr: 240
claimed: 2026-11-08
files: src/index/lambdas.rs, src/index/intentions.rs, src/domain/intentions.rs
---

## 要做什么

接着 `sec35-more-type-intentions`（PR #239）。候选，按日常使用频率排：

- Java：Replace lambda with anonymous class（要知道目标的函数式接口和方法签名）。
- Kotlin：Convert to 'apply' / 'also' / 'run' / 'let'（作用域函数互转）、Replace with 'filterIsInstance'、Add / remove explicit lambda parameter types。
- 先对照 IntelliJ 的意图列表（intellij-community 的 plugins/kotlin 和 java/java-impl 下的 intention / inspection）确认标题和行为，每做一批开一个 PR。

## 进度

- 2026-11-08：这次只做了 Kotlin 作用域函数互转：新文件 `src/index/scope_functions.rs`，`to_receiver`（let/also → run/apply）和 `to_parameter`（run/apply → let/also，接收者类型用 `index::types::type_of`，成员用 `class_members` 从类的声明取）。坑：赋值左边的 `it.age = 3` 解析成 `directly_assignable_expression` 而不是 navigation_expression；字符串模板里的 `$it` 是 `interpolated_identifier`，前面有单独的 `interpolation_identifier_start`。接到 `fixes.rs`，设置里两个 family（`domain::intentions::family`）。
- 剩下的（Java lambda 转匿名类、filterIsInstance、显式 lambda 参数类型）挪到 sec35-more-type-intentions-3。
