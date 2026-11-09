---
id: sec49-multi-caret-shrink
title: 多光标下的 Shrink Selection：每个光标的选区各退回一步
features: 第 49 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec49-multi-caret-shrink
pr:
claimed: 2026-11-09
files: src/ui/file_editor/editing.rs, src/ui/file_editor.rs
---

## 要做什么

照 IntelliJ：多光标时 Extend Selection（Ctrl+W）每个光标各扩大一步，Shrink Selection（Ctrl+Shift+W）按步骤退回，每个光标各退回一步；扩大时合并成一个的选区，退回时恢复成原来的几个。

## 进度
