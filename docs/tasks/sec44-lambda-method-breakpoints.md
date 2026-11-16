---
id: sec44-lambda-method-breakpoints
title: 接口方法断点停在 lambda 实现里
features: 第 43 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec44-lambda-method-breakpoints
pr:
claimed: 2026-11-16
files: src/index/method_breakpoint.rs, src/domain/debugger.rs, src/usecase/debug_session.rs, src/ui/debug_service.rs
---

## 要做什么

FEATURES.md 第 43 节 “接口 / 抽象方法断点在库里和 lambda 的实现” 里的 lambda 部分：在函数式接口的方法上设的断点，也要停在实现它的 lambda 里（IntelliJ 会停）。

不做：库里的实现、方法引用、变量类型靠推断的 lambda，这些留在 FEATURES.md 新的那一行 ⬜。

## 进度

- 2026-11-16：
  - `index::method_breakpoint::taking(lang, text, interface)`：一个文件里有这个接口类型参数的函数 / 构造方法，返回（名字、参数位置）。
  - `index::method_breakpoint::lambdas(lang, text, interface, taking)`：实现接口的 lambda 的起止行（1 起）。Java 认变量 / 字段声明、强转、return、方法调用和 `new` 的实参；Kotlin 认 SAM 构造和调用的实参、尾随 lambda。
  - `usecase::debug_session::Implementation.lambda`：为 true 时 `place` 用 `domain::debugger::lambda_entries`。它按名字认 `lambda$…`、`…$lambda…` 或接口方法名，代码要从 lambda 的行开始，嵌套时只取最外层。
  - `ui::debug_service::implementations`：在 Method Hierarchy 的实现之外扫工程的 .java / .kt。坑：传 lambda 的文件不一定出现接口名（Kotlin 测试里只写 `JCalc.apply(5) { … }`），所以除了含接口名的文件，含 `taking` 函数名的文件也要扫。
  - 测试：`finds_lambdas_implementing_an_interface`、`lambda_breakpoints_stop_in_the_outermost_lambda_on_its_lines`。Linux 实测见 FEATURES.md。
