---
id: sec29-template-functions-2
title: Live Templates：其余少用的表达式函数
features: 第 29 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec29-template-functions-2
pr: 244
claimed: 2026-11-08
files: src/domain/live_templates.rs, src/index/introduce.rs, src/ui/file_editor/templates.rs
---

## 要做什么

docs/FEATURES.md 第 29 节的 ❌ 行：`methodParameterTypes()`、`suggestFirstVariableName()`、`iterableVariable()`、`castToLeftSideType()`、`annotated(…)`、`anonymousSuper()` 等，行为照 intellij-community 里对应的 `*Macro` 类。`groovyScript(…)` 不做（没有 Groovy 运行时）。`descendantClassesEnum` 的第二个参数（不含抽象类）也可以在这里补。

## 进度

- 2026-11-08：domain/live_templates.rs 加 `Computed::{ParameterTypes, IterableVariable, CastToLeftSide, Annotated, AnonymousSuper}`，`Scope.annotated`，`annotation_names()`；parse_simple 认 methodParameterTypes / suggestFirstVariableName / iterableVariable / castToLeftSideType / annotated / anonymousSuper。新 index/annotated.rs `annotated_declarations()`（Java / Kotlin，tree-sitter）；ui/file_editor/templates.rs `annotated_names()` 扫项目源文件（跳过外部库，最多 50 个）。测试 `more_user_template_functions`、`finds_annotated_declarations`。
- 坑：castToLeftSideType 在 IntelliJ 里只在不可赋值时给 `(Type)`，我们没有类型推断，所以总是空。
- 没做：groovyScript（无 Groovy 运行时）、descendantClassesEnum 第二个参数（不含抽象类）；后者另开任务 sec29-template-functions-3。
