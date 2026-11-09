---
id: sec56-session-revision-tabs
title: 重新打开工程时也恢复某个 revision 的文件标签
features: 第 56 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec56-session-revision-tabs
pr: 258
claimed: 2026-11-09
files: src/domain/editor_session.rs, src/ui/workspace/session.rs
---

## 要做什么

照 IntelliJ 重开工程时恢复编辑器：从 Git Log 等处打开的某个 revision 的文件（只读标签）也随会话保存，工程再打开时和工作区文件的标签一起恢复（光标、固定、活动标签、分屏都一样）。现在会话只存工作区文件的标签，revision 标签重启后就没了。

## 进度

- 2026-11-09：做完（PR #258）。`SessionTab` 加 `revision`，`EditorSession` / `SplitSession` 加 `active_revision`（旧会话文件照样能读）；`reopenable` 对 revision 标签不检查工作区文件是否存在，活动标签按路径加 revision 找。`Workspace::session` 不再滤掉 revision 标签；恢复时用新的 `reopen_tab`：工作区文件走 `open_at`，revision 走 `open_file(path, Some(rev))` 再 `go_to`，固定按路径加 revision 找。单测 `reopens_existing_files` 补了 revision 的情况；Linux 实测关掉重开。
