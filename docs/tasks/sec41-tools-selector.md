---
id: sec41-tools-selector
title: Merged Manifest 支持 tools:selector（tools:node 只作用于某个库的元素）
features: 第 41 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec41-tools-selector
pr:
claimed: 2026-11-09
files: src/domain/manifest_merge.rs
---

## 要做什么

照 Android Gradle 插件的清单合并：高优先级清单的元素写了 `tools:selector="<库的 package>"` 时，它的 `tools:node`（remove / removeAll / replace）只作用于那个库（package 或 namespace 等于这个值）合进来的元素，别的库的同名元素照常合并。合并日志（Merging Log）不在这次范围。

## 进度
