---
id: sec42-introduce-param-getters
title: Introduce Parameter：Java 的 Replace fields used in expressions with their getters
features: 第 42 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec42-introduce-param-getters
pr: https://github.com/wizardmly/junction/pull/302
claimed: 2026-11-10
files: src/domain/introduce_parameter.rs, src/index/introduce_parameter.rs, src/ui/introduce_parameter_dialog.rs
---

## 要做什么

照 IntelliJ 的 Java Introduce Parameter 对话框：表达式用到有 getter 的字段时，多一组 “Replace fields used in expressions with their getters” 单选（Do not replace / Replace fields inaccessible in usage context / Replace all fields，默认 Do not replace，记住上次的选择）。调用处的新实参里，按选择把字段换成 getter：`cart.rate` 变成 `cart.getRate()`。“inaccessible” 指私有字段在声明它的顶层类外面、包内可见的字段在别的包里。

## 进度

- 2026-11-10：做完（PR #302）。`ParameterPlan` 加 `fields: Vec<FieldUse>`（表达式里有 getter 的字段：相对位置、getter 名、`Access` Private / Package / Public）、`class_range`（函数所在的顶层类）和 `package`；`index::introduce_parameter::java_fields` 找本类的 field_declaration 和无参 getter（boolean 也认 `isX`），表达式里没被局部变量遮住的裸名字和 `this.x` 都算，赋值的不算。`ParameterRequest.getters: FieldGetters`（None / Inaccessible / All）；`domain::introduce_parameter::argument_at` 和 `call_edit_at` 按 `CallPlace`（在不在顶层类外、调用文件的包，别的文件用 `CallPlace::elsewhere`）决定换不换，换的字段范围里不再加接收者前缀、不再换 `this`。原来的 `call_edit` 去掉（只剩测试在用）。对话框加三项单选，thread_local 记住。测试 `fields_become_getters`、`finds_java_fields_with_getters`；Linux 实测见 FEATURES.md 第 42 节。
- 没做：getter 在父类里（IntelliJ 的 `findMethodBySignature(…, true)` 会查父类）；字段在父类里的情况同样不认。
