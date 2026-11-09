---
id: sec33-reformat-rest-7
title: Reformat：剩下的换行和 Wrapping 页签（第七批）
features: 第 33 节
status: claimed
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-7
pr:
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页（IntelliJ Code Style 的 Wrapping and Braces）。

Imports 页已在 sec33-reformat-rest-6（#253）做完。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
