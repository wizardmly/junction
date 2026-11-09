---
id: sec40-extract-generics-receiver
title: Extract Method 带上所在函数的泛型参数和 Kotlin 扩展函数的 receiver
features: 第 40 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec40-extract-generics-receiver
pr: 252
claimed: 2026-11-09
files: src/domain/extract_method.rs, src/index/extract_method.rs
---

## 要做什么

照 IntelliJ 的 Extract Method：

- 选区里用到所在函数（或方法）声明的类型参数时，新函数也声明这些类型参数（Kotlin `private fun <T> extracted(x: T)`，Java `private <T> void extracted(T x)`），边界原样带上，只带用到的。
- Kotlin 里所在函数是扩展函数（`fun String.foo()`）且选区用到 `this` 或隐式 receiver 时，新函数也做成同一 receiver 的扩展函数，调用处不变（隐式 `this`）。

FEATURES.md 第 40 节最后一行里的“去掉参数、Extract Parameter / Change Signature、按语言服务器取类型、其他文件里的重复片段”不在这次范围。

## 进度

- 2026-11-09：做完（PR #252）。
  - `MethodPlan` 新加 `type_parameters`（按声明原样，如 `T : Comparable<T>`、`T extends Number`）、`type_constraints`（Kotlin `where`）、`receiver`（Kotlin 扩展函数的 receiver 类型）；`domain::extract_method::signature` 把它们写进签名，有 `reified` 时加 `inline`。
  - `index::extract_method::generics`：类型参数在选区里出现（`type_identifier`，或不是本地变量的同名标识符），或在参数、结果、返回类型里出现就带上；用到的参数的边界、`where` 约束里引到的其他类型参数反复补齐。
  - `uses_receiver`：选区里有不在嵌套 lambda / 函数 / 类里的 `this`、`this@所在函数`，或不是本地变量、不是文件里声明的函数 / 属性 / 类、不是大写开头、不是 `it`、不是常见标准库顶层函数（println、listOf…）的名字时，算用到 receiver。
  - `duplicates`：新函数是扩展时只在同一 receiver 的函数里找重复片段。
  - 测试 `keeps_type_parameters_and_receiver`；Linux 实测 `List<T>.largest` 提取成 `private fun <T : Comparable<T>> List<T>.highest(n: Int): T`。
  - 剩下：receiver 成员靠名字猜，准确要靠语言服务器；`this@所在函数` 在新函数里没改成 `this@新函数`。都记在 FEATURES.md 第 40 节最后一行。
