---
id: sec53-recent-locations-follow
title: Recent Locations 的位置按逐行 diff 跟着编辑移动（多光标、整段替换也准）
features: 第 53 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec53-recent-locations-follow
pr: 257
claimed: 2026-11-09
files: src/domain/navigation_history.rs, src/ui/workspace/files.rs, src/ui/workspace/tabs.rs, src/ui/workspace/events.rs, src/ui/workspace/mod.rs
---

## 要做什么

现在 Recent Locations 记下的位置只按编辑前后的行数差和光标所在行推算移动，多光标（几处同时加删行）或整段替换（Reformat、Optimize Imports、重构）时会错。改成和断点一样，按编辑前后文本的逐行 diff（`debugger::follow_lines`）移动：没变的行跟着走，被删的行落到删除处。

## 进度

- 2026-11-09：做完（PR #257）。`RecentPlaces::shift`（按行数差和光标行推算）换成 `RecentPlaces::follow(path, old, new)`，用 `debugger::follow_lines` 按逐行 diff 移动这个文件里的所有位置；工作区的 `line_counts` 换成 `line_texts`（每个打开文件上次编辑后的文本，打开标签时和磁盘重新载入时记下），`Edited` 事件里取新文本、和旧的比。文本没变或这个文件没有位置时不做 diff。单测覆盖加行、删行、两处同时加行；Linux 实测两个光标 Ctrl+D。
