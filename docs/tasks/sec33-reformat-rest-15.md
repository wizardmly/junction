---
id: sec33-reformat-rest-15
title: Reformat：Wrapping and Braces 页剩下的选项（第十五批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-15
pr: 272
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数和局部变量注解、Kotlin 的注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 25 位，旧的照样读，缺的位用语言默认值）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。这个测试也跑第二遍，第二遍有变化就失败（#271）；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：这批做了 Java 的 Local variable annotations 和 Array initializer，其余拆到 sec33-reformat-rest-16。
  - `Wrapping.local_annotations: AnnotationWrap`（IntelliJ 的 VARIABLE_ANNOTATION_WRAP，默认 Do not wrap）：`wrap_annotations` 的括号栈分出类体 / 代码块 / 圆方括号，代码块里注解后面是局部变量（`member_kind` 为 Field）的行按这个选项拆；局部类、`for (@X …)` 里的、带注释的行不动。设置页 `annotation_options` 第五个下拉框。settings.conf 第 26 位。
  - `Wrapping.array_init: Wrap`（ARRAY_INITIALIZER_WRAP，默认 Do not wrap）：`chop` 里 Java 的 `{…}`，前面是 `new T[]` 或 `T[] a =`（`is_array_initializer`），和实参列表一样选最外层、最靠前的拆：第一个元素留在 `{` 后，其余续行缩进，`}` 跟在最后一个后面。设置页 `wrap_option`（Elvis 的下拉框改成通用的）。settings.conf 第 27 位。
  - 顺带改了缩进：Java 数组初始化里换行的元素按续行缩进（`apply_brackets` 的 `java` 参数），照 IntelliJ `ArrayInitializerBlocksBuilder` 的 `Indent.getContinuationWithoutFirstIndent()`。原来按代码块缩进一级。
  - 修了 #271 的一个坑：拆行后再排的那一遍会重新探测缩进单位，新拆出的行可能让它变；现在沿用第一遍的单位。
  - 回归：Signal Java 默认 15746 → 15768。多的 22 行全来自数组初始化的续行缩进：RecentPhotosLoader、Rfc5724UriTest 等 5 个按续行写的文件变少（共少 120 多行），MentionUtilTest、ContactUtilTest 等 6 个手写成缩进一级的文件变多，按 IntelliJ 源码以续行为准。Kotlin 24934 不变。两项都打开时 Java 15800，第二遍 0 行；默认第二遍也 0 行。
  - Xvfb：settings.conf 写入两项后 Reformat Code 拆开数组和局部变量注解；设置页两个下拉框显示保存的值，改成 Wrap if long 后只改了这一位。
