---
id: sec41-tools-selector
title: Merged Manifest 支持 tools:selector（tools:node 只作用于某个库的元素）
features: 第 41 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec41-tools-selector
pr: 260
claimed: 2026-11-09
files: src/domain/manifest_merge.rs
---

## 要做什么

照 Android Gradle 插件的清单合并：高优先级清单的元素写了 `tools:selector="<库的 package>"` 时，它的 `tools:node`（remove / removeAll / replace）只作用于那个库（package 或 namespace 等于这个值）合进来的元素，别的库的同名元素照常合并。合并日志（Merging Log）不在这次范围。

## 进度

- 2026-11-09：做完（PR #260）。`manifest_merge::merge_into` 多一个参数：被合并的库的 package（namespace 或清单的 `package`，和 `tools:overrideLibrary` 用的同一个）；新函数 `node_rule` 返回元素对这个库生效的 `tools:node`（有 `tools:selector` 且不等于 package 时不生效）。remove / removeAll 标记不对这个库生效时，它不是可以合进去的元素：库的元素另外加进来（合并时找同 key 的非标记元素），标记最后照常被 `strip` 去掉。测试 `selector_limits_a_rule_to_one_library`；Linux 实测 app + 两个库。
