---
id: sec33-reformat-rest-13
title: Reformat：Wrapping and Braces 页剩下的选项（第十三批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-13
pr: 269
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如枚举常量（ENUM_CONSTANTS_WRAP）、if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数和局部变量注解、Kotlin 的 Elvis expressions 和注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 23 位，旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这批做了 Java 的 Enum constants，其余拆到 sec33-reformat-rest-14。
  - `Wrapping.enum_constants: AnnotationWrap`（和注解共用四个选项），settings.conf 第 24 位；设置页 `annotation_options` 里第四个下拉框。
  - `wrap_enum_constants`：Java 预处理，在 `wrap_annotations` 之后。括号栈里标出 enum 的类体（`declaration_braces` 认出的类体，且之前最后一个声明关键字是 `enum`），直到体里第一个 `;`；栈顶是这种类体、行里只有圆括号的行按括号外的逗号拆。
  - 回归：Signal Java 15746、Kotlin 23750 不变；设 Wrap always 时 Java 15762，多出的全是拆开的常量，稳定。
