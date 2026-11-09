---
id: sec33-reformat-rest-8
title: Reformat：Do not keep line breaks 和 Wrapping and Braces 页的其他选项（第八批）
features: 第 33 节
status: claimed
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-8
pr:
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行；Keep when reformatting › Line breaks 关闭时合并多余的换行；Wrapping and Braces 页的其他选项（大括号位置、if / for / try 等语句的换行、注解、枚举常量、二元表达式等）。

已有的五项（实参、形参、链式调用、父类列表、赋值）在 sec33-reformat-rest-7（#256）做完，类型是 `domain::reformat::Wrapping`，新选项接着往里加。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
