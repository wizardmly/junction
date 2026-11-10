---
id: sec42-super-getters
title: Introduce Parameter：父类里的字段也按 getter 替换
features: 第 42 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-super-getters
pr: https://github.com/wizardmly/junction/pull/333
claimed: 2026-11-10
files: src/index/introduce_parameter.rs, src/ui/file_editor.rs
---

## 要做什么

Java 的 Replace fields used in expressions with their getters 现在只认本类声明的字段。照 IntelliJ（从字段所在的类找 getter），表达式用到父类声明的字段时也要找到它和 getter。

## 进度

- 2026-11-10：做完。`index::introduce_parameter::plan_with` 多一个按简单类名取类文件文本的参数（`catches::Classes`，编辑器里是 `class_texts`），`plan` 仍是不查别的文件的版本。`java_fields` 先取本类字段（新函数 `class_fields`），再用 `catches::Finder` 沿 `extends` 链找父类（最多 8 层），收进非 private、在同一个类里有 getter 的字段，近的优先；protected 按 `Access::Package`。`file_editor.rs` 的 Introduce Parameter 改调 `plan_with`。测试在 `finds_java_fields_with_getters`。Linux 实测见 FEATURES.md。
- 没做：父类字段的类型不会自动填进 Type（类型推断只看本文件）；getter 在更上层父类而字段在下层时不认；protected 字段在另一个包的子类里其实访问得到，这里按访问不到处理（多换成 getter，结果仍然正确）。
