---
id: sec29-template-functions-3
title: Live Templates：descendantClassesEnum 的“不含抽象类”参数
features: 第 29 节
status: claimed
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec29-template-functions-3
pr:
claimed: 2026-11-08
files: src/domain/live_templates.rs, src/ui/file_editor/templates.rs
---

## 要做什么

`descendantClassesEnum("Base", "true")` 的第二个参数：为 true 时列表里去掉抽象类和接口，照 intellij-community 的 `DescendantClassesEnumMacro`。现在第二个参数被忽略。`groovyScript(…)` 不做（没有 Groovy 运行时）。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
