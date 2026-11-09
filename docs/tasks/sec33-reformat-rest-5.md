---
id: sec33-reformat-rest-5
title: Reformat：剩下的换行和代码风格页签（第五批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-5
pr: 251
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：

- 值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页、Imports 页。
- Java Within 剩下的 Code braces、Angle brackets、Record header、Deconstruction list、Spaces inside block braces when body is present。

Java Within 的其他 17 项和 Unary operator 已在 sec33-reformat-rest-4（#249）做完，`Spaces` 的新项照它的写法加在 `values` 末尾。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

这次只做第二项（Java Within 剩下的五项），换行、Do not keep line breaks、Wrapping、Imports 移到 sec33-reformat-rest-6。

- 2026-11-09：`Spaces.within` 扩到 22 项（values 64..68：Code braces、Angle brackets、Record header、Deconstruction list、Spaces inside block braces when body is present），JAVA 页 Code braces 排在 Within 最前、其余四项在最后（照 IntelliJ）。`java_within()`：`(` 前是 `record X` 归 Record header，`instanceof` / `case` 后的大写类型名（及嵌套模式）归 Deconstruction；`{` 在 `)`（不是 record 头 / 注解之后）、`->`、else / try / finally / do / static 之后算代码块；尖括号单独预扫：`<` 紧跟标识符（或修饰符后的 `<T>`），到配对 `>` 之间只有类型字符、不含 `&&`。测试 `java_within_braces_angles_records`。
- 坑：AS 默认关 Code braces 时会把 `{ return; }` 改成 `{return;}`，Signal 大量写 `{ }`，照“回归不能变多”关时保持原样。
- Signal 语料：改动行数 15766 → 15764（`List<byte[] >` 修好）。
