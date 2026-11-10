---
id: sec43-wildcard-method-breakpoint
title: 按类名 + 方法名的通配方法断点（Breakpoints 对话框 + › Java Method Breakpoints）
features: 第 43 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec43-wildcard-method-breakpoint
pr: https://github.com/wizardmly/junction/pull/298
claimed: 2026-11-10
files: src/usecase/debug_session.rs, src/infra/breakpoint_store.rs, src/ui/debug_service.rs, src/ui/breakpoints_dialog.rs
---

## 要做什么

照 IntelliJ 的 `WildcardMethodBreakpoint`：Breakpoints 对话框的 + 里选 Java Method Breakpoints，填类名模式（`*` 通配，如 `com.example.*Service`）和方法名，不需要源码行；调试时在所有匹配的类（包括库里的类）里同名方法的第一条代码上停。条件、日志、Method entry / exit 等和普通方法断点一样。

不做：lambda 的实现（留在 FEATURES.md 那一行）。

## 进度

- 2026-11-10：完成。
  - 会话（`usecase/debug_session.rs`）：`LineBreakpoint.wildcard: Option<WildcardMethod { class_pattern, method }>`，`LineBreakpoint::wildcard(pattern, method)` 用 “模式.方法名” 当 path、行号 1，所以命中后的条件、日志、依赖、Remove once hit 全走原来行断点的路径。`prepare_patterns()` 给类加载监视的模式（通配断点用 `WildcardMethod::prepare_pattern`：`*` 只在一端时原样，否则 `*`）；`in_packages` 对通配断点按 `glob` 匹配类名；`place` 里按方法名 `glob` 取每个方法的第一条代码，`place_entries`（从 `place_method` 拆出）处理 entry / exit。HotSwap 后通配断点也重新设置。测试 `wildcard_method_breakpoints_stop_in_matching_classes`。
  - 存储：`SavedBreakpoint.class_pattern`，`SavedBreakpoint::wildcard`。
  - 服务：`add_wildcard_method_breakpoint`；`line_breakpoint` 对有 `class_pattern` 的走 `LineBreakpoint::wildcard`。
  - 对话框：+ 改成下拉菜单，`start_adding(method)`；输入按最后一个点拆成模式和方法名；`group` / `title` 认通配断点；View Source 对它不做事。
  - 没做 / 没测：IntelliJ 是两个输入框（Class pattern、Method name），这里是一个；库里的类没单独实测；接口方法断点在库里和 lambda 的实现仍是 ⬜。
