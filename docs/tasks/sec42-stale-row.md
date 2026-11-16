---
id: sec42-stale-row
title: 第 42 节过时的 ⬜ 行
features: 第 42 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-stale-row
pr:
claimed: 2026-11-16
files: docs/FEATURES.md
---

## 要做什么

第 42 节（Introduce Parameter）最后一行 “同名函数的调用按解析区分；Java 的 Change Signature” 还标着 ⬜，其实两件都已经做了。

## 进度

- 2026-11-16：核对代码。`file_editor.rs` 的 Introduce Parameter 用 `our_calls(...)`，也就是 `index::introduce_parameter::OurCalls::keep`，当前文件和其他文件的调用都按索引筛过，和 Change Signature、Inline Function 一样。Java 的 Change Signature 在第 48 节，全部 ✅。只改文档，不改代码。
