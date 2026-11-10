---
id: sec44-hotswap-accessor-breakpoints
title: 调试器：热替换后重新设置 Kotlin 属性访问器上的监视点
features: 第 44 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec44-hotswap-accessor-breakpoints
pr: https://github.com/wizardmly/junction/pull/331
claimed: 2026-11-10
files: src/usecase/debug_session.rs
---

## 要做什么

没有 backing field 的 Kotlin 属性，监视点停在 getter / setter 入口（sec44-kotlin-property-accessors）。Apply HotSwap 重定义类以后，JVM 会清掉旧方法上的断点，这些监视点就不再停。热替换后要像行断点一样在新方法上重设。

## 进度

- 2026-11-10：做完。`DebugSession::redefine` 原来跳过所有字段监视点；现在属性所在源文件被重新加载、且它的请求里有访问器断点（`accessors` 集合）时，也清掉旧请求（访问器的走 `clear_breakpoint`）再 `place` 一遍。按字段 id 设的普通字段监视点重定义后仍有效，照旧不动。测试：`property_watchpoints_without_a_field_stop_in_the_accessors` 里加了热替换的部分。Linux 实测见 FEATURES.md 第 44 节。
