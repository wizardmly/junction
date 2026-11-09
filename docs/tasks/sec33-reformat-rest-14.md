---
id: sec33-reformat-rest-14
title: Reformat：Wrapping and Braces 页剩下的选项（第十四批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-14
pr: 270
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数和局部变量注解、Kotlin 的 Elvis expressions 和注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 24 位，旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这批做了 Kotlin 的 Elvis expressions，其余拆到 sec33-reformat-rest-15。
  - `Wrapping.elvis: Wrap`（Java 不用，`JAVA` 里 Do not wrap；`KOTLIN` 默认 Wrap if long，照 IntelliJ Kotlin 的 ELVIS_WRAP 默认），`Wrapping::any` 也看它。settings.conf 第 25 位，旧文件没这位时用语言默认值。设置页 Kotlin 的 Wrapping and Braces 下 `elvis_option` 一个下拉框（Do not wrap / Wrap if long / Chop down if long）。
  - `elvis_break`：在 `chop` 开头（只 Kotlin），只切括号外（第 0 层）的 ` ?: `，字符串模板 `${…}` 里的不算；续行比语句多一级缩进、`?:` 放行首。Chop 时每个 `?:` 一行，Wrap if long 只在放不下时换。
  - 赋值（`val x = a ?: b`）的值能单独放一行时先不切 Elvis（照旧走赋值换行）；但要是接下来会拆实参列表（`defaultName(` + 换行），且切 Elvis 后每行都在右边界内，就切 Elvis。Signal 里 `?: throw IllegalStateException("…")`、`?: return …` 这类因此和原文一样。
  - 回归：Signal Java 15746 不变、Kotlin 23750 → 23686（多是 `?:` 换行和原文一致）。
  - 发现：语料测试 `formatted_sources_are_stable` 只写结果，不比第二遍。真比后 master 上 Kotlin 默认设置就有 71 个文件 1894 行第二遍还会变（这次之后 1881 行），Java 默认 0 行；Java Binary / Ternary 全开 35 行，注解和枚举常量全开 0 行。记到新任务 reformat-second-pass-stability。
