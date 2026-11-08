---
id: sec29-template-functions-2
title: Live Templates：其余少用的表达式函数
features: 第 29 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/live_templates.rs, src/index/introduce.rs, src/ui/file_editor/templates.rs
---

## 要做什么

docs/FEATURES.md 第 29 节的 ❌ 行：`methodParameterTypes()`、`suggestFirstVariableName()`、`iterableVariable()`、`castToLeftSideType()`、`annotated(…)`、`anonymousSuper()` 等，行为照 intellij-community 里对应的 `*Macro` 类。`groovyScript(…)` 不做（没有 Groovy 运行时）。`descendantClassesEnum` 的第二个参数（不含抽象类）也可以在这里补。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
