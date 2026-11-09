---
id: sec56-session-revision-tabs
title: 重新打开工程时也恢复某个 revision 的文件标签
features: 第 56 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec56-session-revision-tabs
pr:
claimed: 2026-11-09
files: src/domain/editor_session.rs, src/ui/workspace/session.rs
---

## 要做什么

照 IntelliJ 重开工程时恢复编辑器：从 Git Log 等处打开的某个 revision 的文件（只读标签）也随会话保存，工程再打开时和工作区文件的标签一起恢复（光标、固定、活动标签、分屏都一样）。现在会话只存工作区文件的标签，revision 标签重启后就没了。

## 进度
