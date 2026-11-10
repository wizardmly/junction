---
id: sec42-introduce-param-java-options
title: Introduce Parameter：Java 的 Declare final 和 Delegate via overloading method
features: 第 42 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-introduce-param-java-options
pr: https://github.com/wizardmly/junction/pull/300
claimed: 2026-11-10
files: src/domain/introduce_parameter.rs, src/index/introduce_parameter.rs, src/ui/introduce_parameter_dialog.rs
---

## 要做什么

照 IntelliJ 的 Java Introduce Parameter 对话框：

- “Declare final”：新参数写成 `final int amount`。
- “Delegate via overloading method”：照 `IntroduceParameterProcessor.generateDelegate`，在方法前面加一个原签名的重载，方法体是 `return total(price, price * rate);`（void 方法没有 return，构造方法用 `this(…)`），调用处都不改；原方法的 `@Override` 去掉。

字段改 getter（Replace fields used in expression with their getters）不在这次范围。

## 进度

- 2026-11-10：做完（PR #300）。`ParameterRequest` 加 `declare_final`、`delegate` 和 `keeps_calls()`（Kotlin 默认值或 Java 委托时调用处不改，file_editor 改用它判断）；`ParameterPlan` 加 `delegate: Option<DelegateSite>`，由 `index::introduce_parameter::delegate_site` 从 Java 的 method / constructor 声明算出：声明起点、到方法体 `{` 为止的头部（含注解和修饰符，不含前面的 Javadoc）、缩进和缩进单位（方法体第一行比声明多出的部分，取不到时 4 个空格）、是否有返回值、是否构造方法、`@Override` 的位置（连同后面的空白）。`domain::introduce_parameter::edits` 在委托时把重载插在声明前，去掉原方法的 `@Override`，不改调用。对话框（`introduce_parameter_dialog`）Java 时显示两个勾选框，Delegate 只在方法有方法体时出现，选择用 thread_local 记住。测试 `java_final_and_delegate`、`finds_where_a_java_delegate_goes`；Linux 实测见 FEATURES.md 第 42 节。
- 没做：varargs 参数时 IntelliJ 把新参数放在 varargs 前面（这里所有情况都加在最后，原来就是这样）；字段改 getter 留在 FEATURES.md 第 42 节的 ⬜ 行。
