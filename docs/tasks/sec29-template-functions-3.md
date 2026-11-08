---
id: sec29-template-functions-3
title: Live Templates：descendantClassesEnum 的“不含抽象类”参数
features: 第 29 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec29-template-functions-3
pr: 246
claimed: 2026-11-08
files: src/domain/live_templates.rs, src/ui/file_editor/templates.rs
---

## 要做什么

`descendantClassesEnum("Base", "true")` 的第二个参数：为 true 时列表里去掉抽象类和接口，照 intellij-community 的 `DescendantClassesEnumMacro`。现在第二个参数被忽略。`groovyScript(…)` 不做（没有 Groovy 运行时）。

## 进度

- 2026-11-08：查 IntelliJ 源码发现“含抽象类”是第三个参数（默认 true），第二个参数是列表里用全名还是短名（我们插入时本来就缩短，不区分）。`Computed::Descendants(base, concrete)`，`Scope.descendants` 带上 concrete 标记；ui/file_editor/templates.rs `concrete_subclass_names()` 沿继承链找子类（抽象类本身不列，但继续找它的子类），`is_abstract_or_interface()` 看符号类型和声明行里名字前的 abstract / sealed / interface。测试 `abstract_classes_and_interfaces`、`user_template_expressions` 补的断言。Xvfb：rt 模板 `descendantClassesEnum("Base", "true", "false")` 列出 Child / Kid / Dog，不含 abstract Pet。
- 坑：修饰符只看声明那一行，修饰符换行写的情况会漏判。
