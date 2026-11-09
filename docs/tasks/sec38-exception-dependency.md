---
id: sec38-exception-dependency
title: 异常断点参与依赖（Disable until hitting the following breakpoint）
features: 第 38 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec38-exception-dependency
pr: https://github.com/wizardmly/junction/pull/265
claimed: 2026-11-09
files: src/usecase/debug_session.rs, src/infra/breakpoint_store.rs, src/ui/debug_service.rs, src/ui/breakpoints_dialog.rs
---

## 要做什么

照 IntelliJ 的 Breakpoints 对话框：异常断点也有 “Disable until hitting the following breakpoint” 和 After hit，下拉框里可以选行断点或异常断点；行断点的下拉框里也能选异常断点作为 master。异常断点命中（过滤和条件都通过）后，依赖它的断点生效。

不做：异常断点按文件分组（留在 FEATURES.md 那一行）。

## 进度

- 2026-11-09：完成。
  - 会话（`usecase/debug_session.rs`）：新增 `BreakpointKey { Line(path, 1 起的行), Exception(类名) }`，`BreakpointActions.master` 和 `ExceptionBreakpoint.master` 都是它，`ExceptionBreakpoint` 加 `leave_enabled`；`State::waiting` / `State::hit` 统一处理“等 master”和“命中后让依赖者生效、自己按 Disable again 重新等待”。异常断点在过滤和条件之前先看是否在等 master，过滤和条件都通过后才算命中。测试 `exception_breakpoints_take_part_in_dependencies`（行断点 → 异常断点 → 行断点 的链）。
  - 存储（`infra/breakpoint_store.rs`）：`BreakpointRef` 加可选的 `exception`（`file` / `line` 改成可缺省，旧文件照常读），`is_line` / `is_exception` 判断；`SavedExceptionBreakpoint` 加 `master`、`leave_enabled`。测试 `masters_are_lines_or_exceptions`。
  - 服务（`ui/debug_service.rs`）：`forget_master` 在删掉行断点或异常断点时解除两种依赖者的依赖（运行中的会话同步）；`follow_edit` 时异常断点的 master 行也跟着移动；`master_key` 把存储的引用转成会话的 key。
  - 对话框（`ui/breakpoints_dialog.rs`）：`render_master` / `set_master` 由行断点和异常断点的属性面板共用，下拉框列出除自己外的所有行断点和异常断点。
  - 坑：Xvfb 上 Breakpoints 对话框打开时，点在对话框里某处有一次落到了后面编辑器的行号栏，多出一个行断点；没查是不是 Xvfb 才有。
  - 没做：异常断点按文件分组（FEATURES.md 第 38 节单独一行）。
