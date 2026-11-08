---
id: sec35-more-type-intentions
title: Alt+Enter 更多需要类型的意图（lambda 反向转换、作用域函数等）
features: 第 35 节
status: open
owner:
branch:
pr:
claimed:
files: src/index/lambdas.rs, src/index/intentions.rs, src/domain/intentions.rs
---

## 要做什么

接着 `sec35-type-intentions`（PR #235，`src/index/lambdas.rs`）。候选，按日常使用频率排：

- Java：Replace lambda with anonymous class（要知道目标的函数式接口和方法签名）、Replace method reference with lambda。
- Kotlin：Convert reference to lambda（`User::name` → `{ it.name }`）、Convert to 'apply' / 'also' / 'run' / 'let'（作用域函数互转）、Replace with 'filterIsInstance'、Add / remove explicit lambda parameter types。
- 先对照 IntelliJ 的意图列表（intellij-community 的 plugins/kotlin 和 java/java-impl 下的 intention / inspection）确认标题和行为，每做一批开一个 PR。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
