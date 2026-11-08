---
id: sec16-instrumented-debug
title: 仪器测试配置的 Debug 和 Deployment Target Options
features: 第 16 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec16-instrumented-debug
pr:
claimed: 2026-11-08
files: src/ui/run_service.rs, src/ui/run_config_dialog.rs, src/domain/run.rs
---

## 要做什么

docs/FEATURES.md 第 16 节：Android Instrumented Tests 运行配置的 Debug（现在提示不支持）和 Deployment Target Options。容器里只能用模拟 adb 验证，FEATURES.md 里注明需要真机验证。

## 进度

- 2026-11-08 认领。先做 Debug（照 AS：装好 app 和测试 APK 后 `am instrument -e debug true`，连上被测进程），Deployment Target Options 视情况放到后面。
