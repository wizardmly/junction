---
id: sec16-deployment-target
title: 仪器测试配置的 Deployment Target Options
features: 第 16 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec16-deployment-target
pr:
claimed: 2026-11-08
files: src/ui/run_config_dialog.rs, src/domain/run.rs, src/ui/run_service.rs
---

## 要做什么

docs/FEATURES.md 第 16 节：Android Instrumented Tests 运行配置的 Deployment Target Options（AS 的 Target：Use the device/snapshot drop down、Specific devices 等），让 Run 和 Debug 按配置选设备。Debug 已在 sec16-instrumented-debug（#248）做完，按标题栏选中的设备跑（`TestRequest.device` → `ANDROID_SERIAL`），可以照它把配置里的设备传下去。容器里只能用模拟 adb 验证，FEATURES.md 里注明需要真机验证。

## 进度

- 2026-11-08 认领。Android App 和 Android Instrumented Tests 两种配置都加 Deployment Target Options（Use the device/snapshot drop down / Specific devices）。
