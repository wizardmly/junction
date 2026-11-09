---
id: sec44-object-field-watch
title: Variables 里对字段 Add Field Watchpoint（只监视这个对象）
features: 第 44 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec44-object-field-watch
pr: https://github.com/wizardmly/junction/pull/267
claimed: 2026-11-09
files: src/ui/debug_view.rs, src/ui/debug_service.rs, src/index/method_breakpoint.rs
---

## 要做什么

照 IntelliJ：调试暂停时，在 Variables 里右键对象的字段 → Add Field Watchpoint，在这个字段的声明行设字段监视点，Instance filters 填上这个对象（只在这个对象的字段被写时停）。已有监视点时把对象加进它的 Instance filters。

不做：没有 backing field 的 Kotlin 属性（自定义 getter），留在 FEATURES.md 那一行。

## 进度

- 2026-11-09：完成。
  - `index::method_breakpoint`：抽出 `fields(lang, text)`（文件声明的字段和行），`field_at` 用它，新增 `field_line(lang, text, name)`；测试加在 `finds_fields`。
  - `DebugService::field_of_object(path)`：Variables 的一行是不是对象的字段（父节点是 `Value::Object`），返回对象 id、类名、字段名；`add_field_watchpoint(path)`：按类名（去掉 `$内部类`）拼 `包/类名.java`、`.kt` 交给 resolver 找文件，`field_line` 找声明行；没有断点时用 `kind_on` 建字段监视点并带 `instances: [id]`，有的话把 id 加进 `instances`（走 `set_breakpoint_properties`，会话里同步）；失败写进 Debug 控制台。
  - `DebugView::render_variables`：对象字段的行有右键菜单 “Add Field Watchpoint”（图标 breakpoint-field）。
  - 坑：继承来的字段声明在父类文件里，现在只找对象自己的类的文件，找不到时只在控制台提示。
