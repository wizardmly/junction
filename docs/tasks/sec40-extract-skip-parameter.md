---
id: sec40-extract-skip-parameter
title: Extract Method / Function：不传某个参数
features: 第 40 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec40-extract-skip-parameter
pr: https://github.com/wizardmly/junction/pull/347
claimed: 2026-11-16
files: src/domain/extract_method.rs, src/ui/extract_method_dialog.rs, src/ui/file_editor.rs, src/index/extract_method.rs
---

## 要做什么

FEATURES.md 第 40 节 ⬜ 行里的 “去掉参数”：IntelliJ 的参数表每行有勾选框，取消勾选的参数不传。

## 进度

- 2026-11-16：
  - 查了 IntelliJ 源码。Java 的 `ExtractMethodProcessor` 对 `passAsParameter == false` 的参数，在 `defineVariablesForUnselectedParameters()`（默认 true）时，在新方法体开头加 `类型 名字 =;`。Kotlin K2 的 `KotlinFirExtractFunctionDialog.createNewDescriptor` 只保留勾选的参数，用法的替换不变。
  - 代码改动：
    - `ExtractRequest.skipped`（计划里的参数下标）。`ordered()` 会去掉这些参数，所以签名、调用和重复片段的实参都跟着变。
    - `extract` 在 Java 方法开头写 `{ty} {name} = ;`。Kotlin 被赋值的参数如果没传，不再写 `var x = x`。
    - 对话框每行加 `Checkbox`，字段是 `passed`。
    - `file_editor.rs` 在有参数没传时不找重复片段。
  - 测试：`leaves_out_unchecked_parameters`。Linux 实测见 FEATURES.md。
