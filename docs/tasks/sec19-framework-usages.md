---
id: sec19-framework-usages
title: Android 资源：框架资源（android.R / @android:）的 Find Usages 和翻译
features: 第 19 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec19-framework-usages
pr: 247
claimed: 2026-11-08
files: src/index/, src/usecase/android_resources.rs
---

## 要做什么

docs/FEATURES.md 第 19 节的 ❌ 行：在 `android.R.color.white` / `@android:color/white` 上 Find Usages 列出项目里所有用到它的地方；框架字符串的翻译（Translations Editor 不列框架资源，照 AS 看要不要做）。现在框架资源只能跳到声明。

## 进度

- 2026-11-08：domain/android_resources.rs 新 `framework_usage_pattern()`；index/service.rs `framework_resource_at()`（引用处或 SDK res 目录里的声明），`usages()` 先查它，`resource_usages()` 改成传入 pattern；`go_to()` 在 SDK 声明名字上返回 `Nav::Declaration`（弹出用处）。测试 `file_resources_and_patterns` 补了框架的断言。Xvfb：假 SDK（/tmp/claude-0/fakeadb）+ 带 compileSdk 的 build.gradle.kts 验证。
- 翻译不做：AS 的 Translations Editor 只列模块资源。
- 坑：项目资源的 Find Usages 正则 `\bR\.color\.x` 也会匹配 `android.R.color.x`（regex 没有后行断言），同名时项目资源的结果里会混进框架引用，没改。
