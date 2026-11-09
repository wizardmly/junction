---
id: sec53-recent-locations-follow
title: Recent Locations 的位置按逐行 diff 跟着编辑移动（多光标、整段替换也准）
features: 第 53 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec53-recent-locations-follow
pr:
claimed: 2026-11-09
files: src/domain/navigation_history.rs, src/ui/workspace/files.rs, src/ui/workspace/tabs.rs, src/ui/workspace/events.rs, src/ui/workspace/mod.rs
---

## 要做什么

现在 Recent Locations 记下的位置只按编辑前后的行数差和光标所在行推算移动，多光标（几处同时加删行）或整段替换（Reformat、Optimize Imports、重构）时会错。改成和断点一样，按编辑前后文本的逐行 diff（`debugger::follow_lines`）移动：没变的行跟着走，被删的行落到删除处。

## 进度
