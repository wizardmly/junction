---
id: sec42-introduce-param-java-options
title: Introduce Parameter：Java 的 Declare final 和 Delegate via overloading method
features: 第 42 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-introduce-param-java-options
pr:
claimed: 2026-11-10
files: src/domain/introduce_parameter.rs, src/index/introduce_parameter.rs, src/ui/introduce_parameter_dialog.rs
---

## 要做什么

照 IntelliJ 的 Java Introduce Parameter 对话框：

- “Declare final”：新参数写成 `final int amount`。
- “Delegate via overloading method”：照 `IntroduceParameterProcessor.generateDelegate`，在方法前面加一个原签名的重载，方法体是 `return total(price, price * rate);`（void 方法没有 return，构造方法用 `this(…)`），调用处都不改；原方法的 `@Override` 去掉。

字段改 getter（Replace fields used in expression with their getters）不在这次范围。

## 进度
