---
id: sec16-instrumented-debug
title: 仪器测试配置的 Debug 和 Deployment Target Options
features: 第 16 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec16-instrumented-debug
pr: 248
claimed: 2026-11-08
files: src/ui/run_service.rs, src/ui/run_config_dialog.rs, src/domain/run.rs
---

## 要做什么

docs/FEATURES.md 第 16 节：Android Instrumented Tests 运行配置的 Debug（现在提示不支持）和 Deployment Target Options。容器里只能用模拟 adb 验证，FEATURES.md 里注明需要真机验证。

## 进度

- 2026-11-08 认领。先做 Debug（照 AS：装好 app 和测试 APK 后 `am instrument -e debug true`，连上被测进程），Deployment Target Options 视情况放到后面。
- 2026-11-08 完成 Debug（#248）：`TestRequest.device`（串号）和 `TestPlan.env`，仪器测试带 `debug_port` 时 `plan` 加 runner 参数 `debug=true`、设 `ANDROID_SERIAL`；`TestService::set_debug_device` 让下一次运行只在该设备上跑，`last_module` 给出模块和变体；`DebugService::debug_on_device`（`debug_tests` 多了 `instrumented` 参数，gutter、Project 窗口、临时配置、保存的配置四个入口都走它）先记下 app 已有的 pid，然后每 0.5 秒用 `output-metadata.json` 的 applicationId 查 `pidof`，出现新 pid 就用 `android::jdwp::connect` 连上，测试结束或 5 分钟没等到就结束；设备来自工作区注入的 `DeviceProvider`（标题栏选中的在线设备）。坑：`gradle::apk_output` 要求 APK 文件存在，第一次构建前查不到 applicationId，会一直等到构建出 APK。验证：模拟 adb（Python 写的 adb server，`jdwp:<pid>` 转发到本机 `-agentlib:jdwp` 的 JVM）加假的 `connectedDebugAndroidTest` Exec 任务。Deployment Target Options 移到 sec16-deployment-target。
