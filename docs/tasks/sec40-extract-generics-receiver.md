---
id: sec40-extract-generics-receiver
title: Extract Method 带上所在函数的泛型参数和 Kotlin 扩展函数的 receiver
features: 第 40 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec40-extract-generics-receiver
pr:
claimed: 2026-11-09
files: src/domain/extract_method.rs, src/index/extract_method.rs
---

## 要做什么

照 IntelliJ 的 Extract Method：

- 选区里用到所在函数（或方法）声明的类型参数时，新函数也声明这些类型参数（Kotlin `private fun <T> extracted(x: T)`，Java `private <T> void extracted(T x)`），边界原样带上，只带用到的。
- Kotlin 里所在函数是扩展函数（`fun String.foo()`）且选区用到 `this` 或隐式 receiver 时，新函数也做成同一 receiver 的扩展函数，调用处不变（隐式 `this`）。

FEATURES.md 第 40 节最后一行里的“去掉参数、Extract Parameter / Change Signature、按语言服务器取类型、其他文件里的重复片段”不在这次范围。

## 进度
