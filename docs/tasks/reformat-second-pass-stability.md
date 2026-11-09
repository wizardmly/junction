---
id: reformat-second-pass-stability
title: Reformat Code 第二遍不变（Kotlin 默认、Java Binary / Ternary）
features: 第 33 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/reformat.rs
---

## 要做什么

格式化过的代码再按同样设置格式化一次应该一个字都不变（IntelliJ 就是这样）。现在不是：

- Signal Kotlin 语料（200 个文件，默认设置）：第一遍的结果再跑一遍，71 个文件约 1880 行还会变。多是多行实参 / lambda 收尾的续行缩进（`testDispatcher` + `) {` 之后的体多缩进一级，`group.recipientId` + `)` 多两格）。
- Signal Java 语料（400 个文件）：默认设置 0 行；Binary expressions / Ternary operation 打开（Chop 或 Wrap if long 都一样）时 35 行会变，链式调用 `.isCallFull(false)` 和 `||` 续行的缩进第二遍又加深。注解、枚举常量 Wrap always 时 0 行。

另外 `formatted_sources_are_stable`（`#[ignore]` 的语料测试）名字叫 stable，其实只把结果写到 `JUNCTION_FMT_OUT`，不比第二遍。改成也跑第二遍并报告（或断言）有变化的行，这样以后的格式化改动能直接看到。

做法：先给测试加第二遍比较，再挑最常见的几类不稳定按 IntelliJ 的缩进规则修，Kotlin 改动行数不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
