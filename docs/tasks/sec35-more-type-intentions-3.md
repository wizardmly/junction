---
id: sec35-more-type-intentions-3
title: Alt+Enter 更多需要类型的意图（第三批：lambda 转匿名类、filterIsInstance 等）
features: 第 35 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec35-more-type-intentions-3
pr: 242
claimed: 2026-11-08
files: src/index/lambdas.rs, src/index/intentions.rs, src/domain/intentions.rs
---

## 要做什么

接着 `sec35-more-type-intentions-2`（PR #240，作用域函数互转）。候选：

- Java：Replace lambda with anonymous class（要知道目标的函数式接口和方法签名）。
- Kotlin：Replace with 'filterIsInstance'（`filter { it is T }.map { it as T }`）、Add / remove explicit lambda parameter types；`with(x) { … }` 和 `x.run { … }` 互转。
- 先对照 IntelliJ 的意图列表（intellij-community 的 plugins/kotlin 和 java/java-impl 下的 intention / inspection）确认标题和行为，每做一批开一个 PR。

## 进度

- 2026-11-08 完成（PR #242），都对照了 intellij-community 的测试数据（plugins/kotlin/idea/tests/testData/inspectionsLocal/scopeFunctions、collections/simplifiableCall、intentions/specifyExplicitLambdaSignature）：
  - `src/index/scope_functions.rs`：`scope_call` 改成返回 `ScopeCall`，也认 `with(x) { … }`；`head()` 负责在 `x.f {` 和 `with(x) {` 两种写法之间改头部；`to_receiver` 现在返回 Vec（let 同时给 run 和 with），新增 `with_and_run`（run ↔ with），`to_parameter` 也接受 with。
  - `src/index/simplifiable_call.rs`：`simplify_call`，filter → filterIsInstance / filterNotNull，mapNotNull → filterIsInstance，flatMap { it } → flatten（元素类型用 `lambdas::collection_element`，已改成 `pub(super)`）。
  - `src/index/lambda_signature.rs`：`specify_signature`、`remove_parameter_types`；坑：tree-sitter 里解构参数的类型 `(a, b): T` 是 `lambda_parameters` 下和 `multi_variable_declaration` 并列的节点，不在它里面。
  - `src/index/lambda_to_anonymous.rs`：`lambda_to_anonymous`，JDK 接口表 `known()` + 项目接口 `declared()`，`substitute()` 替换类型参数；`lambdas::line_indent` / `reindent` 改成 `pub(super)` 复用。
  - 接线：fixes.rs 的 Kotlin / Java 意图列表，domain/intentions.rs 的 FAMILIES 和 `family()`（“Convert 'x' call to 'y'” 归到 “Simplify library function call”），设置搜索关键字。
  - 测试：每个文件有单测；Xvfb 打开 Kotlin / Java 文件逐个 Alt+Enter 应用。
  - 没做（够小就不再开任务，需要时再加）：lambda 签名的返回类型 / 接收者类型和任意函数的参数类型；作为方法参数的 Java lambda 转匿名类（要解析重载）；filterIsInstance 的类型相交检查。
