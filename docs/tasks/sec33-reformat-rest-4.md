---
id: sec33-reformat-rest-4
title: Reformat：剩下的换行和代码风格页签（第四批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-4
pr: 249
claimed: 2026-11-08
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：

- Java 的 Within 组（括号、方括号等，默认都关，现在固定去掉括号内的空格）和 Java 的 Around operators › Unary operator（默认关）。
- 值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页、Imports 页。

Kotlin 的 In simple one line methods、Range、Unary 已在 sec33-reformat-rest-3（#245）做完，`Spaces` 的新项照它的写法加在 `values` 末尾。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

这次只做第一项（Java 的 Within 组和 Unary operator），剩下的换行、Do not keep line breaks、Wrapping、Imports 移到 sec33-reformat-rest-5。

- 2026-11-08：`Spaces.within: [bool; 17]`（values 47..63，`Within` 枚举的顺序），JAVA 页加 44（Unary）和 47..63。domain/reformat.rs 的 `normalize()` 对 Java 行最后跑 `java_within()`：先标出代码字符（跳过字符串、字符、注释），按括号配对，`paren_kind()` 看 `(` 前面判断 if/for/…/注解/方法声明/方法调用/类型转换/分组。一元运算符沿用 Kotlin 的 `unary()`，Java 也走，加了 `~`、`case` / `yield` / `assert` / `default`，类型转换后的 `-x`（抽出 `sign_after_paren()`）。测试 `java_within_and_unary_spaces`。
- 坑：Signal 的数组初始化都写 `{ 1 }`，AS 默认会去掉；照“回归不能变多”，Array initializer braces 关时保持原样。续行开头的 `+ b`（Java）不当一元运算符。
- Signal 语料：/tmp/claude-0/sig 里 400 个 Java 文件，改动行数 15766 → 15766。
