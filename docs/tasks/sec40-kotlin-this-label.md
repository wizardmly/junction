---
id: sec40-kotlin-this-label
title: Kotlin Extract Function：this@原函数名 改成新函数名
features: 第 40 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec40-kotlin-this-label
pr:
claimed: 2026-11-16
files: src/index/extract_method.rs, src/domain/extract_method.rs
---

## 要做什么

FEATURES.md 第 40 节 ⬜ 那一行里的 “`this@函数名` 在新函数里没改名”：在扩展函数里提取用到 `this@原函数名` 的代码时，新函数是同一 receiver 的扩展，但标签还是原函数名，编译不过。应改成 `this@新函数名`。

不做：同一行里的去掉参数、Extract Parameter / Change Signature、按语言服务器取类型、其他文件里的重复片段。

## 进度

- 2026-11-16：`MethodPlan.receiver_labels`（代码里 `this@原函数名` 的标签部分，相对代码的位置），`index::extract_method::plan` 在新函数带 receiver 时找出来，`domain::extract_method::extract` 和参数改名一起换成新名字。测试加在 `keeps_type_parameters_and_receiver`（`with(n) { this@shout.repeat(this) }` 变成 `this@g`，lambda 的 `this` 不动）。Linux 实测见 FEATURES.md。
