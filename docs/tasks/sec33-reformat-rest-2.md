---
id: sec33-reformat-rest-2
title: Reformat：剩下的换行和代码风格页签（第二批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec33-reformat-rest-2
pr: 243
claimed: 2026-11-08
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行、Do not keep line breaks、Code Style 设置页剩下的（Spaces 的 Within、Kotlin 的 Before left brace 等、Wrapping、Imports）。Java 的 Before left brace / In ternary operator 已在 sec33-reformat-rest（#236）做完。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归，改动行数不能变多。

## 进度

这次先做 Code Style › Spaces 页剩下的选项。换行（Wrapping）、Do not keep line breaks、Imports 页留给后面。

- 2026-11-08（Claude / Megan，PR #243）：先做了 `->` 两边的空格，因为 Reformat 原来完全不动箭头（`{x->x}`、`1->a()` 原样留着），而 IntelliJ 默认都会加空格。`Spaces::arrows`（4 个，`SPACES_LEN` 40 → 44）：Java 的 Lambda arrow，Kotlin 的 Around arrow in function types / Around arrow in 'when' clause / Before lambda arrow。`reformat::arrow_spaces` 判断是哪种箭头：前面是 `(…)` 且这对括号不是紧跟在 `{` 后的解构，就当函数类型；行里有没闭合的 `{`，就当 lambda；其余算 when 分支。限制：lambda 的 `{` 在上一行、参数和箭头单独在下一行时，会被当成 when 箭头（两个设置都默认开，结果一样）。
  语料回归：corpus 测试多了 `JUNCTION_FMT_OUT`，把结果写到目录里，方便用 diff 数改动行数（原来按行号逐行对比，换行一插入后面整片都算改动）。Signal 最大的 60 个 .kt，结果和改动前一字不差（与原文不同的 diff 行都是 16776）。
  注意：任务描述里写的“Kotlin 的 Before left brace”，IntelliJ 的 Kotlin Spaces 页其实没有这一组。Kotlin 页剩下的是 Range operator（`..`，默认关）、Unary operator（默认关）、In simple one line methods 和 lambda 花括号内的空格（`{x}` → `{ x }`，默认开，现在不加）。
- 剩下的拆到 sec33-reformat-rest-3：Kotlin 花括号内空格、Range / Unary operator，Java 的 Within 组（默认都关，现在固定去掉括号内空格），Wrapping，Do not keep line breaks，Imports 页。

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
