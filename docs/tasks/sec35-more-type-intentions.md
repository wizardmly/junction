---
id: sec35-more-type-intentions
title: Alt+Enter 更多需要类型的意图（lambda 反向转换、作用域函数等）
features: 第 35 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec35-more-type-intentions
pr: 239
claimed: 2026-11-08
files: src/index/lambdas.rs, src/index/intentions.rs, src/domain/intentions.rs
---

## 要做什么

接着 `sec35-type-intentions`（PR #235，`src/index/lambdas.rs`）。候选，按日常使用频率排：

- Java：Replace lambda with anonymous class（要知道目标的函数式接口和方法签名）、Replace method reference with lambda。
- Kotlin：Convert reference to lambda（`User::name` → `{ it.name }`）、Convert to 'apply' / 'also' / 'run' / 'let'（作用域函数互转）、Replace with 'filterIsInstance'、Add / remove explicit lambda parameter types。
- 先对照 IntelliJ 的意图列表（intellij-community 的 plugins/kotlin 和 java/java-impl 下的 intention / inspection）确认标题和行为，每做一批开一个 PR。

## 进度

- 2026-11-08：这次做了两个反向转换：Kotlin Convert reference to lambda（`index::lambdas::reference_to_lambda`，声明来自 `kotlin_member` / `kotlin_class_member` / `kotlin_constructor`，`place_lambda` 把最后一个实参移到括号外）和 Java Replace method reference with lambda（`method_reference_to_lambda`，`java_method` 取参数名，`JAVA_KNOWN` 内置 JDK 常用方法，`unused_name` 避开重名）。坑：tree-sitter-kotlin 把顶层的 `User::name` 和 `this::show` 解析成带 `::name` 后缀的 navigation_expression，不是 callable_reference，两种都要认。已接到 `ui/file_editor/fixes.rs` 和 `domain::intentions::FAMILIES`。
- 剩下的候选挪到 sec35-more-type-intentions-2。
