---
id: sec42-introduce-param-getters
title: Introduce Parameter：Java 的 Replace fields used in expressions with their getters
features: 第 42 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-introduce-param-getters
pr:
claimed: 2026-11-10
files: src/domain/introduce_parameter.rs, src/index/introduce_parameter.rs, src/ui/introduce_parameter_dialog.rs
---

## 要做什么

照 IntelliJ 的 Java Introduce Parameter 对话框：表达式用到有 getter 的字段时，多一组 “Replace fields used in expressions with their getters” 单选（Do not replace / Replace fields inaccessible in usage context / Replace all fields，默认 Do not replace，记住上次的选择）。调用处的新实参里，按选择把字段换成 getter：`cart.rate` 变成 `cart.getRate()`。“inaccessible” 指私有字段在声明它的顶层类外面、包内可见的字段在别的包里。

## 进度
