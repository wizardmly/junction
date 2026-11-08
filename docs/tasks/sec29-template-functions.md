---
id: sec29-template-functions
title: Live Templates：剩下的表达式函数
features: 第 29 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec29-template-functions
pr: 237
claimed: 2026-11-08
files: src/domain/live_templates.rs, src/domain/template_xml.rs, src/ui/file_editor/templates.rs
---

## 要做什么

docs/FEATURES.md 第 29 节的 ❌ 行：complete() / completeSmart()、methodReturnType()、descendantClassesEnum(…) 等。groovyScript(…) 要跑 Groovy 脚本，先不做，在 FEATURES.md 里写明原因。

## 进度

- 2026-11-08：做了 `complete()` / `completeSmart()` / `classNameComplete()`、`methodReturnType()`、`descendantClassesEnum(…)`、`filePath()`、`fileRelativePath()`。`domain::live_templates::Computed` 加 `Complete`、`MethodReturnType`、`FilePath`、`FileRelativePath`、`Descendants(base)`；`Scope` 加 `return_type`（`index::introduce::template_scope` 填）、`file_path` / `relative_path`、`descendants`（编辑器展开前按 `descendant_bases` 用 `subclass_names` 查好放进去，`descendant_choices` 给停留时的列表）；`completion_variables` 标出要开补全的变量，`ui/file_editor/templates.rs` 的 `TemplateState.complete` 记着，停到它时 `complete_if_asked` 调 `trigger_completion`。Xvfb 实测见 FEATURES.md 第 29 节。
- 没做：groovyScript（没有 Groovy 运行时，不做）；其余少用的函数挪到 sec29-template-functions-2。
