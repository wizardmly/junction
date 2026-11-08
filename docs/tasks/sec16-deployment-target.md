---
id: sec16-deployment-target
title: 仪器测试配置的 Deployment Target Options
features: 第 16 节
status: open
owner:
branch:
pr:
claimed:
files: src/ui/run_config_dialog.rs, src/domain/run.rs, src/ui/run_service.rs
---

## 要做什么

docs/FEATURES.md 第 16 节：Android Instrumented Tests 运行配置的 Deployment Target Options（AS 的 Target：Use the device/snapshot drop down、Specific devices 等），让 Run 和 Debug 按配置选设备。Debug 已在 sec16-instrumented-debug（#248）做完，按标题栏选中的设备跑（`TestRequest.device` → `ANDROID_SERIAL`），可以照它把配置里的设备传下去。容器里只能用模拟 adb 验证，FEATURES.md 里注明需要真机验证。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
