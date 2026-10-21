# Junction Studio 交接文档

写给接手这个仓库继续开发的人或 AI。读完这一份，再按需看 [docs/FEATURES.md](docs/FEATURES.md)（逐项功能状态）和 [docs/DIFF_MERGE.md](docs/DIFF_MERGE.md)（Diff/Merge 规格），就能接着做。

最后更新：2026-10-20，对应 master `cdf5451`。

---

## 1. 项目是什么

**Junction Studio — Git & Code Navigator**：一个独立的桌面客户端（Windows / macOS / Linux），目标是：

1. **一比一复刻 Android Studio（IntelliJ 新 UI）的 Git 功能**：Log、Commit 工具窗口、Diff/Merge、分支弹窗、Rebase、Shelf/Stash、多仓库根、GitHub/GitLab 等，细节、菜单、快捷键、图标都尽量和 AS 一致。
2. **复刻 AS 的代码索引和跳转**：Kotlin、Java、C/C++、Objective-C、Rust、Swift、Dart、Go、V、JS/TS、Python，外加跨语言桥接（JNI、Dart FFI、Rust `extern "C"`、Swift/ObjC、cgo）和外部库源码。
3. 后来扩展到 Android 日常工具：Terminal、Device Manager、Running Devices（投屏）、Device Explorer。

**判断标准始终是“和 Android Studio 一样”**。不确定某个行为该怎样时，以 AS（IntelliJ Platform 源码 `intellij-community`，以及 AS 实际表现）为准，而不是自己设计。

几条已经定下来的决定：

- 名字：显示名 `Junction Studio`（窗口标题、About、macOS `Junction Studio.app`）；crate / 可执行文件 / 设置目录 / 环境变量仍叫 `junction` / `Junction` / `JUNCTION_*`。旧名 GitGlass 的设置会在首次启动时迁移（`git::migrate_legacy_data`）。
- 外观：保持现在的 IntelliJ 新 UI 风格。早期提过“玻璃”效果，但**不是需求**。
- 技术栈：**Rust + GPUI + gpui-component（Longbridge，Apache 2.0）**。否决过的方案：Zed 插件（没有自定义 UI 能力）、fork Zed（GPL-3）、剥离 git4idea（耦合太深）、Compose + Jewel（备选第二）。
- **不要复制 Zed 的代码**（GPL）。可以参考 IntelliJ（Apache 2.0）。
- Git 后端和 git4idea 一样，直接调用 `git` 命令行，不用 libgit2。
- 索引：自有 tree-sitter 索引始终可用，本机装了语言服务器就叠加 LSP；跨语言桥接由自有索引解析。

## 2. 当前状态一览

里程碑全部完成（详见 FEATURES.md 文末“阶段”）：

| 阶段 | 内容 | 状态 |
|---|---|---|
| M1–M5 | 窗口骨架、Log、Commit、Diff、分支、Rebase、冲突合并、Blame、部分提交、Shelf、补丁、多仓库根、Worktree、Submodule | ✅ |
| M6 | GitHub / GitLab：Token 登录、Pull Requests 工具窗口、评论审查、Share / Gist / Create PR、Clone 列出仓库 | ✅（OAuth 登录未做） |
| M7 | 代码索引：13 种语言符号、桥接、LSP、Go to Declaration、Find Usages、Go to File/Class/Symbol、External Libraries | ✅ |
| M8 | 搜索：Find/Replace in Files、Search Everywhere、Find Action、Recent Files、File Structure、编辑器标签页与分屏 | ✅ |
| M9 | AS 细节对齐：图标、实现/覆盖 gutter、Ctrl+点击、弹窗拖动、Find/Terminal 工具窗口、树的选择行为 | ✅ |
| M10 | Android：Device Manager、Wi-Fi 配对、Running Devices、Device Explorer | ✅（只在 Linux 上用模拟 adb 测过，真机只验证过部分） |

质量相关：

- 2026-10-09 做过一次全量回归（358 项对照 AS），之后全部修复并复测：旧问题 127/135 通过，0 崩溃。
- 2026-10-12 做过一次“所有右键菜单 / 弹出菜单 / 主菜单每一项”的回归：首轮 518 项通过、42 个问题，已全部修复并复测两轮。
- `cargo test` 当前 441 个单元测试通过（另有 15 个 `#[ignore]` 的基准、探针和需要外部工具的测试，如调用 keytool 的 `creates_a_keystore`）。
- 绝大多数功能**只在 Linux（Xvfb）下测过**。Windows 上验证过的：启动、任务栏点击最小化、旧索引缓存崩溃修复、Device Manager 的 Wi-Fi 自动连接，以及维护者在 Windows 11（10.0.26200，Gradle JDK 用 AS 自带的 JBR 21.0.9）上编译 mytv-android 的 APK。macOS 基本未在真机验证。
- CI 每次推分支都在 `windows-latest` 上跑一次 `cargo check --release`，只在 Windows 下编译的代码（`#[cfg(windows)]`）在合并前就能发现编译错误，但不等于在 Windows 上跑过。

## 3. 怎么编译、运行、测试

### 编译运行

需要 Rust 1.90+ 和 `git`。

```sh
cargo run --release -- /path/to/repo     # 不传路径时打开当前目录
cargo test                               # 单元测试
```

- **Linux 依赖**：`libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libvulkan-dev libfontconfig-dev libssl-dev`。在无 GPU 的容器里跑还要 `mesa-vulkan-drivers`（否则 “Failed to create surface”）。
- **Windows**：MSVC Build Tools。`vendor/gpui-pre-windows` 打过补丁：Release 版不再需要 `fxc.exe`，着色器在运行时用 D3DCompile 编译。升级 gpui-kit 时保留这个 `[patch.crates-io]`。
- **macOS**：需要完整 Xcode（Metal 着色器编译器）。`packaging/build-macos-app.sh --install` 生成并安装 `Junction Studio.app`。
- 第一次编译依赖约 10–15 分钟。
- 在 Linux 上交叉检查 Windows 代码能否编译：`rustup target add x86_64-pc-windows-gnu`，装 `mingw-w64`，然后 `CARGO_TARGET_DIR=target-win cargo check --release --target x86_64-pc-windows-gnu`。

### 发布

`.github/workflows/release.yml`：**每次推送到 master** 自动编译 Windows zip、Linux tar.gz + AppImage、macOS universal .dmg + .app.zip（ad-hoc 签名，未公证），并替换 Release [`latest`](https://github.com/wizardmly/junction/releases/tag/latest)。所以推 master 就是发版，推之前务必确保能编译、测试通过。macOS runner 上 `hdiutil` 偶尔报 “Resource busy”，脚本里已有重试。

- **发布不用 Actions 构件**（2026-10-20 起）：`prepare` 先建草稿 Release `build-<run id>`（顺手删掉被取消 / 失败的运行留下的 `build-*` 草稿），三个平台编译完用 `gh release upload --clobber` 直接传到草稿，最后的 `release` 任务下载全部文件、加 `SHA256SUMS.txt`，删掉旧的 `latest`，再把草稿改成 `latest` 发布。原因：以前用 `upload-artifact` 中转，构件默认留 90 天，加上 CI 每次推送的 `linux-ci` 构件，仓库的构件存储额度被占满（6.6 GB），10-19 / 10-20 两次发版在上传那步失败、什么也没发布。GitHub 每 6–12 小时才重算一次用量，删了构件也不会马上恢复。**不要再在发布流程里用构件中转。**
- `.github/workflows/ci.yml`：分支和 PR 上跑 Linux 编译（数警告）+ 单元测试，Windows `cargo check`；`linux-ci` 构件只留 1 天，上传失败（额度满）不算 CI 失败。
- **提交流程**：AI 会话在分支 `claude/testing-tool-window-j0gvx7` 上开发、推送，CI 绿了以后提 PR 到 master，由维护者合并（合并即发版）。合并后把分支重置到最新 master（`git checkout -B <分支> origin/master`）再继续，不要在已合并的历史上叠提交。
- 仓库里的 `.claude/settings.json` 允许 Claude Code 会话执行 `git push origin *:master`（维护者 2026-10-18 加的，从新开的会话起生效）。有了它，做完一项、CI 通过就可以直接推 master，不必再走 PR；AI 不能自己修改这个文件（会被当成给自己放权而拦下）。

### 在无界面环境下看 UI

项目一直是这样验证 UI 的：

- `Xvfb :99 -screen 0 1600x1000x24 &`，`DISPLAY=:99` 启动程序，`xdotool` 点击和输入，`import -window root shot.png` 截图。
- 每次测试用一个干净的 `XDG_CONFIG_HOME` / `XDG_CACHE_HOME`，避免旧设置影响。
- 坑：
  - Xvfb 收不到“单独按修饰键”的事件，**双击 Shift 测不了**，用标题栏搜索按钮或 Ctrl+N 代替。
  - xdotool 的 Ctrl+点击不可靠，测跳转用 **Ctrl+B**。
  - `JUNCTION_THEME=light` 会被已保存的设置覆盖，要改主题就先改 `~/.config/Junction/settings.conf` 里的 `theme=`。
  - 软件渲染（llvmpipe）下帧时间偏高，不能当真实性能看。
  - 不要 `pkill -f release/junction`（会杀掉自己的 shell），用 `kill $(pgrep -x junction)`。
- 重构时用过像素对比：同一仓库、同一步骤，旧二进制和新二进制各截一张，`compare -metric AE -fuzz 2%`。

### 基准和探针（`src/bench.rs`，都是 `#[ignore]`）

```sh
JUNCTION_BENCH=<repo> cargo test --release bench_backend -- --ignored --nocapture
# 其他：bench_index_stages, bench_symbol_search, bench_diff_switch, bench_gutter,
#       bench_index_load, bench_libraries, bench_multi_root_log
JUNCTION_PROBE=<repo> cargo test probe_project -- --ignored --nocapture   # 也有 probe_jumps
```

用过的测试仓库：Linux 内核（`git clone --filter=blob:none`，148 万提交）、AOSP `frameworks/base`（`--depth 1`，Java/Kotlin 继承关系）、[nowinandroid](https://github.com/android/nowinandroid)（README 截图）。

性能分析：主线程卡顿用 gdb 批量采样（`thread apply 1 bt`）配合 xdotool 操作最有效；也可以用 `perf`。

### 测试 GitHub / GitLab 功能（不连外网）

- `JUNCTION_GITHUB_API=http://127.0.0.1:PORT/api/v3` 指向本地 mock API，配合 `NO_PROXY=127.0.0.1`，在 `~/.config/Junction/accounts.json` 写一个假 token。
- git 传输用 `GIT_SSH_COMMAND` 指向一个脚本，在本地裸仓库目录里执行 upload-pack / receive-pack。

### 测试 Android 功能（没有真机）

用一个假的 `adb` 脚本（放在 PATH 前面）模拟 `adb devices`、`shell` 输出、`mdns services` 等；Running Devices 用一个假的 scrcpy 服务端（Python 脚本推送 ffmpeg 生成的 H.264 流并记录控制消息）。Device Explorer 的 Processes 需要假 adb 带一个 JDWP 服务端。

容器里没有 Android SDK，AGP 构建跑不起来。模块类型是从 build 脚本文本判断的，所以一个只有 `settings.gradle.kts`（`include(":app")`）和 `app/build.gradle.kts`（`plugins { id("com.android.application") }`、`buildTypes { release { } }`）的假工程就能让 Run 配置、Build Variants、Generate Signed Bundle / APK 对话框认出 app 模块；要验证传给 Gradle 的参数，把 app 的 build 脚本临时换成一个注册同名任务（如 `assembleRelease`）并打印 `providers.gradleProperty(...)` 的脚本。

## 4. 代码结构

约 7 万行 Rust。分层方向：**UI → model → git / index / hosting / android**，反方向没有依赖。

```
src/main.rs         启动、窗口、崩溃处理（crash.rs）
src/model.rs        仓库状态中心：后台加载、多根（RootState）、run_task / run_operation 后台执行框架
src/settings.rs     设置读写（config_dir()：Linux ~/.config/Junction，Windows %APPDATA%\Junction，macOS ~/Library/Application Support/Junction）
src/theme.rs        IntelliJ Int UI 亮/暗配色、编辑器高亮配色 editor_colors()
src/git/            纯 git CLI 封装，不依赖 GPUI：log 解析、graph（提交图）、refs、status、diff、merge、
                    rebase、patch、blame、roots（多根映射）、worktree、submodule…；test_support.rs 提供 TestRepo
src/index/          代码索引：symbols（tree-sitter 抽取）、bridge（跨语言桥接）、lsp、libraries（外部库发现
                    与缓存）、nav（跳转排序）、hierarchy（继承/实现）、locals（局部变量作用域）、store（缓存）、
                    service（后台调度）、text_search
src/hosting/        GitHub / GitLab REST 客户端（同一 Client，gitlab 标志区分）
src/android/        adb、avd、sdk 定位、wifi（Wi-Fi 设备逻辑）、pairing、mirror（scrcpy 会话）、
                    text_input（设备端写字小程序）、files、process、jdwp
src/terminal.rs     终端后端（alacritty_terminal，Windows 用 ConPTY）
src/spell.rs        提交信息拼写检查（assets/dict）
src/bookmarks.rs    书签（~/.config/Junction/bookmarks.json）
src/bench.rs        基准与探针
src/ui/             界面
  workspace/        主窗口：tabs、navigation、search、files、panels、chrome、menu、vcs、events、session
  log_view/         Git Log：filter_bar、branches、table、details、go_to、commit_menu
  commit_view*      Commit 工具窗口
  diff_view*, text_panes.rs, diff_panes.rs, merge_view.rs   Diff / Merge（共用 TextPanes）
  dialogs/          各种对话框
  device_manager/, running_devices/, device_explorer/        Android 工具窗口
  as_icons.rs, as_icons_gen.rs                              AS 图标（生成文件）
vendor/             打过补丁的 gpui-component、gpui-base、gpui-pre-windows
assets/             as-icons、highlights（自写 tree-sitter 查询）、dict、scrcpy、text-input、图标
tools/              as_icons.py（重新生成图标）、text-input（设备端小程序源码与 build.sh）
packaging/          macOS Info.plist 与打包脚本、Linux .desktop
docs/               FEATURES.md、DIFF_MERGE.md、截图
```

### 新功能的分层（Clean 架构，2026-10-14 起）

新功能按四层写，依赖只能从外往里：`ui → usecase → domain ← infra`。

- `src/domain/`：纯数据和规则，以及“端口” trait（`CodeModel`、`SyntaxChecker`、`BuildSystem`…）。不依赖 GPUI、进程、文件或索引的存储。
- `src/usecase/`：一个功能一个用例（`complete_code`、`check_syntax`、`compile_on_save`），只通过 domain 的端口工作，用假的端口实现写单元测试。
- `src/infra/`：端口的实现（`index_code_model` 包装现有索引，`tree_sitter_syntax`，`gradle` 调 gradlew 并解析编译器输出）。
- `src/ui/`：界面和组装（`completion.rs` 是补全的 CompletionProvider，`compile_service.rs` 是保存后编译的全局服务）。

现有的 `git/`、`index/`、`android/` 暂不迁移，新功能需要它们时在 `infra/` 里包一层适配器。

- 依赖规则由 `src/arch_tests.rs` 检查（`cargo test` 里跑）：`domain` 不能用 GPUI、文件 / 进程 / 网络和外层模块；`usecase` 不能用 GPUI、`ui`、`infra`；`infra` 不能用 GPUI、`ui`、`usecase`。`#[cfg(test)]` 的部分不算。
- 端口放在 `domain`：例如 `domain::project_files::ProjectFiles`（Rename、Introduce Parameter、Change Signature 改其他文件都经过它，实现是 `infra::project_files::DiskFiles`）、`domain::editor_session::SessionStore`（实现 `infra::editor_session_store::JsonSessions`）。跨文件改调用点的流程在 `usecase::update_calls`。
- 导航历史（Back / Forward、Recent Files、Recent Locations 的位置和代码片段）是 `domain::navigation_history`，编辑器会话的数据是 `domain::editor_session`，`ui/workspace/session.rs` 只做组装。
- 读文件不要放在 UI 线程：Call Hierarchy 的分组和读文件在 `index::call_hierarchy::group_by_function`，由 `CodeIndex::callers_of` / `callees_of` 在后台做；Recent Locations 在后台读没打开的文件。
- 所有端口都在 `domain`（`ResFiles` 也已移过去），`infra` 只依赖 `domain`（测试除外），由 `arch_tests` 检查。
- 2026-10-17 之后新加的：`domain::signing`（签名请求、校验、`SigningStore` 记住 keystore 路径和别名、`KeyTool` 新建 keystore；实现 `infra::signing_store::JsonSigning`、`infra::keytool::JdkKeyTool`），`domain::ansi`（控制台 ANSI 解码），`usecase::project_edits`（在其他 Kotlin / Java 文件里找并统一改写，Inline Property 用），`domain::run::TaskRunner::run_tasks` 多了 Gradle 属性参数（`-Pname=value`）。UI 需要的端口实现在组装处（`ui/workspace/mod.rs` 的 action 处理）创建后传进去，UI 组件本身不直接调 `infra`。
- 还没迁移的旧依赖（新代码不要照抄）：`index/service.rs`、`index/libraries.rs` 直接调用 `infra::gradle_sync`、`infra::catalog_files`、`usecase::android_resources`；部分 UI（断点、Gradle 下载）直接调用 `infra` 的存储函数。行为不确定时查源码：编辑器、补全、Problems、Local History、测试、Debugger 看 intellij-community；Logcat、Build/Run、Gradle Sync、资源、设备相关看 AOSP `platform/tools/adt/idea`（都是 Apache 2.0）。

### 需要知道的约定和坑

- **UI 目录拆分方式**：`workspace/`、`log_view/`、`dialogs/` 等目录里，子文件各写一个 `impl X` 块，方法是 `pub(super)`，开头 `use super::*`。新功能按这个方式加到对应子文件。
- **不要在 UI 里直接拼 git 命令**：业务逻辑放到 `git/` 里写成可测试的函数（用 `TestRepo` 写测试），UI 通过 `model.run_task` / `run_operation` 在后台执行。不要在主线程同步跑 git。
- **单个函数不超过 300 行**：render 函数拆成 `render_*` 辅助方法（方法需要 `&mut Context` 又和其他 `cx` 用法并列时返回 `AnyElement`）。
- **缓存视图**：面板是 GPUI 的 cached view（`ui::common::cached`），`Workspace::redraw_panels` 在 model / index 变化时通知它们。**一个缓存面板如果 render 里读了新状态，必须 observe 它并 `cx.notify()`**，否则界面不刷新。
- **索引缓存兼容**：`ProjectIndex` / `LibraryCache` 的**第一个字段必须是 `version: u32`**，一律通过 `index::store::read_cache` 读（先校验版本，再按文件大小限制 bincode 解码）。**改了缓存结构就把 `CACHE_VERSION` 加一**。曾经因为旧缓存被新结构解码，Windows 启动时申请了几 EB 内存直接崩溃。
- 缓存位置：项目索引 `<gitdir>/junction/index.bin`；外部库索引 `~/.cache/Junction/libraries`（跨项目共享）；jar 源码解压到 `~/.cache/Junction/sources`。外部库文件以绝对路径作为键，只读打开。
- **vendor 补丁**：`vendor/` 下的改动都标有 `Junction:` 注释（对话框居中不滑入、通知右下角堆叠且最旧在下、通知 10 秒自动消失、分层彩色图标、按钮尺寸和悬停色、补全插入 import 和括号内光标、补全项带 command 时发 `InputEvent::CompletionCommand`（选中模板后展开）、Ctrl+Space 手动补全 `trigger_completion`、导出 Diagnostic 等）。升级 gpui-kit 时要从 registry 重新拷贝这些 crate，再把 `Junction:` 的改动补回去。
- **图标**：全部用 IntelliJ 官方新 UI SVG（Apache 2.0，见 `assets/as-icons/NOTICE.md`）。源文件放 `assets/as-icons/src/<名字>.svg` 和 `<名字>_dark.svg`（从 `JetBrains/intellij-community` 的 `platform/icons/src/expui/...` 取），然后 `python3 tools/as_icons.py` 重新生成 `assets/as-icons/gen` 和 `src/ui/as_icons_gen.rs`。代码里 `use crate::ui::as_icons as icons;`，直接传 `icons::X`。不要再引入别的图标库。
- **语法高亮**：`src/ui/file_editor.rs` 的 `language_for(path)` 映射扩展名，`grammars()` 注册自己的查询覆盖工具包自带的（工具包里 Swift、CMake、proto、C# 等的查询是空的）。**`.v` 是 V 语言，不是 Verilog**。测试 `every_language_highlights` 检查每种语言的关键字颜色，加语言时要加样例。
- **多仓库**：`model.rs` 里每个根一个 `RootState`；`route(path)` / `run_in_roots` 按文件所在的根执行 git；Log 合并各根历史（`git::log::merge_logs`）。判断提交属于哪个根要用 `rev-parse <hash>^{commit}`，因为 `rev-parse --verify <40位hex>` 对任意 40 位哈希都返回成功。macOS 上 `git rev-parse --show-toplevel` 返回真实路径，而 `RootState.path` 是拼出来的，有符号链接时路径比较会失败（疑似导致 mac 上切换仓库后 Project 树不刷新，未确认）。
- **Running Devices 输入**：非 ASCII 文字（中文等）由 `assets/text-input` 里的小程序以 shell 身份通过 UiAutomation 直接写进焦点输入框；失败时退回 Gboard Unicode 组合输入，再不行就丢弃并通知。**绝对不要用剪贴板粘贴**（会弹系统提示，明确不接受）。手机上如果有其他程序占着 UiAutomation（例如 Maestro、其他 instrumentation），小程序起不来，通知里会列出占用者。
- **设置和日志**：Linux `~/.config/Junction/settings.conf`；Windows `%APPDATA%\Junction\`，启动崩溃看 `stderr.log`（内存分配失败、panic），`crash.log` 只记 SEH 异常。
- 编译警告基线：约 9 个已有的 dead-code 警告，改动不要新增警告。
- **Windows 上运行 Gradle**：直接 `Command::new("gradlew.bat")`，不要再套 `cmd /C`。Rust 标准库会按 cmd 的规则给 .bat 的参数转义，签名密码里的 `&`、`^`、`%`、引号才能原样传到 Gradle。Gradle 不认名字里带点的 `ORG_GRADLE_PROJECT_*` 环境变量，所以签名属性只能走 `-P`（AS 也是这样）。
- **缺 DLL 的诊断**：`main.rs` 启动时 `SetErrorMode(SEM_FAILCRITICALERRORS)`，子进程继承，缺 DLL 的程序不再弹系统框而是以 0xC0000135 退出；`domain::build::missing_dll` 认出 Gradle 输出里的这个退出码并说明是哪个程序。每次构建的第二行写 Gradle JDK 和 Windows 版本（`infra::gradle::environment`）。维护者报过的 `api-ms-win-eventing-consumer-l1-1-0.dll` 弹框：已排查 junction.exe、aapt2、platform-tools、Temurin 17/21、JBR 21.0.8/21.0.9、Gradle 8.11 的原生库，都不依赖它；加了上面的诊断后，维护者贴出的一次 release 构建输出里没有缺 DLL 的提示（那次构建因签名文件不存在失败）；弹框是否还会出现、是哪个程序，都还没确认。
- **控制台**（`ui/console_view.rs`）：按滚动位置决定是否跟随新输出（vendor 编辑器加了 `scrolled_to_end`），往上滚就停住；插入文本前后要恢复滚动位置，否则编辑器会滚到光标处。Build / Run / Debug 走 `set_lines`（ANSI 解码、1 MB cycle buffer 在这里），Logcat 走 `sync`，有自己的缓冲区。
- **密码库**（`domain::password_safe::PasswordSafe`，实现 `infra::password_safe::SystemPasswordSafe`，对应 IntelliJ 的 `PasswordSafe`）：macOS 钥匙串（`security-framework`）；Linux 先用 `secret-tool`（Secret Service，密码走 stdin），没有 D-Bus / 钥匙环时退回设置目录的 `passwords.json`（0600）；Windows 用 `passwords.json`，值用 DPAPI 加密。读写可能弹出解锁框，**一律放在后台线程**。目前只有签名对话框的 Remember passwords 在用；GitHub / GitLab token 仍是明文 `accounts.json`，以后可以迁过来。
- **Inline**（Ctrl+Alt+N）的顺序：局部变量（`index::inline`，只认函数体里的属性）→ 成员属性 / 字段（`index::inline_field`，没有类型解析，只跟随不会认错的引用，遇到 `other.x` 就拒绝）→ 函数（`index::inline_function`）。

### 环境变量

| 变量 | 用途 |
|---|---|
| `JUNCTION_THEME` | `light` / `dark`（会被已保存的设置覆盖） |
| `JUNCTION_GITHUB_API` | 覆盖 GitHub API 根地址（mock 测试） |
| `JUNCTION_CACHE_DIR` | 覆盖外部库索引的缓存目录 |
| `JUNCTION_BENCH*`、`JUNCTION_PROBE*` | 基准和探针参数（见 `src/bench.rs`） |
| `JUNCTION_ASKPASS`、`JUNCTION_REBASE_TODO` | 内部使用：git 凭据回调、交互式 rebase 编辑器 |

## 5. 参与贡献

- **流程**：fork 仓库，从 master 拉分支开发，提 Pull Request 到 `wizardmly/junction` 的 master。用你自己的 git 身份提交即可。master 每次合入都会触发 GitHub Actions 自动发版，所以 PR 合入前要能编译、测试通过。
- **提 PR 前自查**：`cargo build` 无新增警告、`cargo test` 全过；改了界面的，附上截图（和 AS 同一界面的截图放在一起对比最好）。
- **对齐 AS 是第一原则**：新功能先查清 AS 的行为、文案、图标、快捷键，再实现；PR 里说明对照的是 AS 的哪个界面。
- **语言**：文档用中文；代码注释和提交信息用英文（参考 `git log` 的风格：一句话说清用户能看到的变化，常以 “… like Android Studio” 结尾）。
- 功能完成后同步更新 `docs/FEATURES.md` 对应行（✅ / 🟡 / ❌ 加括号说明细节），Diff/Merge 相关更新 `docs/DIFF_MERGE.md`。
- 只在部分平台测过的功能，在 FEATURES.md 里注明测过的平台，例如“Linux 实测，Windows 需真机验证”。

## 6. 还没做的事（待办清单）

### 6.1 Git / 编辑器的剩余缺口

| 项 | 现状 |
|---|---|
| GitHub / GitLab OAuth 浏览器登录 | 只支持 Token。需要先注册 OAuth App 拿 client id，由维护者决定 |
| 多仓库：子仓库文件的部分操作 | ✅ 2026-10-21：Create Patch、合并冲突工具、Project 窗口右键 Git 菜单在嵌套仓库里都走文件所在的仓库（已在嵌套仓库合并冲突的项目里核对）；多仓 Apply Patch 支持选 changelist；外层 `.gitignore` 忽略的嵌套仓库不再在 Project 窗口重复列出文件和 `.git` |
| Log 里 AS 的合并引用标签（“origin & main” 叠放） | ✅ 2026-10-21（FEATURES.md 2.2 节） |
| 编辑器多行标签、双击标签最大化编辑器 | ✅ 2026-10-21 |
| 同一文件在两个分屏里共享一份编辑缓冲 | ✅ 2026-10-21（两边同步文本和保存状态；撤销历史各自一份） |
| 菜单里置灰的 STUB 项 | Find Usages、Move/Copy（F6/F5）、Reformat/Optimize Imports、Module Settings、Local History（含目录 / 项目历史和 Recent Changes）、Mark Directory As › Excluded、Repair IDE on File、Override File Type、Analyze（Inspect Code / Code Cleanup / Analyze Dependencies）都已接上（2026-10-21） |
| jar 没有源码时的类文件骨架 + “Download Sources” 横幅 | ✅ 2026-10-16（FEATURES.md 第 47 节） |
| 实现/覆盖标记的继承关系 | ✅ 2026-10-21：Java/Kotlin 同名类型按 import、包、`.*` 和 `Outer.Name` 写法解析（Signal 上 271 个 `Job.Factory`、17 个 `Constraint.Factory` 实现与源码一致）；其他语言仍按简单名；匿名对象（`object : X.Callback`）不算 |
| 不打算做 | Project 窗口的 Edit Scopes、Group Tabs、View Mode、Move to、Resize、Remove from Sidebar，Android / Packages 视图（Junction 没有对应的 IDE 布局和 Android 模块模型） |

### 6.2 用 Junction 替代 AS 做日常 Android 开发的缺口（2026-10-13 评估）

现在能替代 AS 的：Git、看代码和跳转、全局搜索、终端、Device Manager、投屏、Device Explorer。还不能替代的核心是下面 P0 四项，缺了每天都得回 AS。

**P0：不做就无法日常开发**

| # | 功能 | 现状 | 实现思路 |
|---|---|---|---|
| 1 | 编辑器语言智能 | ✅ 2026-10-14 按“A+B”方案完成：索引补全 + 自动 import + 语法错误 + 保存后编译报错（FEATURES.md 第 14 节）。不常驻 Kotlin 语言服务（省 1–2 GB）；kotlin-lsp 以后可做成设置里的可选开关 | Reformat Code、Parameter Info、Alt+Enter 的 Import class 和意图操作已补上（2026-10-16，FEATURES.md 第 33–35 节）；需要类型信息的快速修复随语言服务器（FEATURES.md 第 37 节，2026-10-16 已用 kotlin-lsp 实测） |
| 2 | Gradle Sync | ✅ 2026-10-14 轻量版完成：init script 解析各模块 classpath 并下载缺的依赖和源码，外部库按结果精确列出；build 文件变化时自动同步，打开项目只提示不同步（FEATURES.md 第 15 节） | Build 窗口 Sync 标签页已补上（2026-10-20）；Build Variants 随第 3 项完成 |
| 3 | Build + Run | ✅ 2026-10-15 完成：标题栏 Make / 设备 / 运行配置 / Run / Stop，Build、Run、Build Variants 工具窗口（FEATURES.md 第 16 节） | Edit Configurations 对话框已补上（2026-10-16，FEATURES.md 第 16 节）；Apply Code Changes 以 HotSwap 方式补上（2026-10-20，FEATURES.md 第 60 节：调试中通过 JDWP 重载类，Android 用 d8 转 dex，未真机验证；不调试时没有 AS 的部署代理，改为重装重启）；剩真机验证（Debug 见第 5 项） |
| 4 | Logcat | ✅ 2026-10-15 完成：Logcat 工具窗口（Alt+6），设备下拉、AS 过滤语法、AS 配色、PROCESS STARTED / ENDED、堆栈跳源码、Standard / Compact、暂停、清空、重启（FEATURES.md 第 17 节） | 过滤输入补全、历史和收藏已补上（2026-10-16）；多个 Logcat 标签页已补上（2026-10-16）；截屏录屏按钮已补上（2026-10-16）；分屏已补上（2026-10-20）；剩真机验证 |

**工程配置**：✅ Gradle JDK（2026-10-15，FEATURES.md 第 23 节）：用户在 Windows 上遇到 “JAVA_HOME is not set and no 'java' command could be found”，现在按 AS 的规则自动找 JDK（含 Toolbox 安装的 AS），并有 Settings › Gradle 的 Gradle JDK 下拉。Download JDK 已补上（2026-10-16）。Gradle user home、Offline work、每个构建的 Distribution（Wrapper / 指定的 Gradle 安装目录）已补上（2026-10-20，FEATURES.md 第 23 节），顺带修了设置页改的 Gradle JDK 点 Apply 后不保存的问题。Project Structure 对话框（SDK Location、AGP / Gradle 版本、模块的 Properties 和 Default Config）已补上（2026-10-20，FEATURES.md 第 61 节）；Dependencies 页和 Build Variants 页已补上（2026-10-20）；Signing Configs 标签和新增未写的属性已补上（2026-10-21）；新增 / 删除构建类型、风味、签名配置和 Suggestions 页已补上（2026-10-21）。

**P1：每天会用，可暂时回 AS**

5. 🟡 Debugger（2026-10-15，FEATURES.md 第 22 节）：JDWP 客户端、行断点（持久化、类加载后补设）、Debug JVM 单元测试、Debug 'app'（`am start -D` + adb `jdwp:<pid>`）、Device Explorer 的 Attach debugger、执行点、Debug 工具窗口（F9/F8/F7/Shift+F8/Alt+F9、Frames、Variables 树）。**Android 调试只在模拟 adb 下验证过，需真机 / 模拟器验证**。条件断点、求值输入框、Watches、toString / 集合显示已补上（2026-10-15）。日志断点（Suspend execution、Log 选项、Shift+点行号）和 Disconnect（VirtualMachine.Dispose）已补上（2026-10-16）。Evaluate Expression 对话框（Alt+F8）和 Breakpoints 对话框（Ctrl+Shift+F8）已补上（2026-10-16，FEATURES.md 第 38 节）。Java 异常断点和按 diff 跟随编辑的断点已补上（2026-10-16，第 38 节）。方法断点（方法入口，2026-10-16，FEATURES.md 第 43 节）已补上。字段监视点（2026-10-16，FEATURES.md 第 44 节）和 Mute Breakpoints（第 45 节）已补上。方法退出（Method exit，2026-10-20，第 43 节）已补上。求值的 new、类型转换、instanceof、赋值已补上（2026-10-20，第 22 节）；数组创建已补上（2026-10-20，第 22 节）；剩求值里的 lambda（需要先编译成类再定义到 VM 里，暂不做）。
6. 🟡 符号级重构：✅ Rename（Shift+F6，2026-10-15，FEATURES.md 第 24 节）：局部变量按作用域，类 / 方法 / 属性按项目整词匹配并列出每处供取消勾选，注释和字符串可选，类改名连带文件 `git mv`。✅ Optimize Imports（Ctrl+Alt+O，FEATURES.md 第 25 节）。✅ Introduce Variable（Ctrl+Alt+V，FEATURES.md 第 26 节，照 IntelliJ 的 KotlinIntroduceVariableHandler / KotlinNameSuggester）。✅ Introduce Constant（Ctrl+Alt+C，FEATURES.md 第 27 节，照 KotlinIntroduceConstantHandler 及其测试数据）。Rename 的语义匹配和 Alt+Enter 的服务器代码操作已接上语言服务器（2026-10-16，FEATURES.md 第 37 节；Kotlin 需安装 kotlin-lsp，已在 Kotlin/JVM 工程上实测 Rename 和 Import 快速修复）。✅ Extract Method / Function（Ctrl+Alt+M，2026-10-16，FEATURES.md 第 40 节）。✅ Introduce Parameter（Ctrl+Alt+P，2026-10-16，FEATURES.md 第 42 节）。✅ Change Signature（Ctrl+F6，2026-10-16，FEATURES.md 第 48 节）。
7. ✅ Android 资源（2026-10-15，FEATURES.md 第 19 节）：`R.x.y` / `@x/y` 跳转、悬停显示值、Find Usages、颜色和图片 gutter 预览。Translations Editor 已补上（2026-10-16，第 31 节，单个 res 目录）。vector drawable 预览已补上（2026-10-16，第 32 节）。shape / selector / layer-list 等 drawable 预览已补上（2026-10-16，第 39 节）。Merged Manifest 已补上（2026-10-16，第 41 节，本地模块 + debug 覆盖，未含 AAR）。
8. ✅ 测试（2026-10-15，FEATURES.md 第 21 节）：gutter ▶ / ▶▶ 和结果图标、Ctrl+Shift+F10、Gradle test 任务 + init script 实时事件 + JUnit XML、Run 窗口的测试标签页（结果树、状态行、Rerun Failed Tests、控制台）。Project 窗口右键运行目录 / 文件的测试已补上（2026-10-20，FEATURES.md 第 21 节）。断言对比窗口（`<Click to see difference>`）已补上（2026-10-20）。运行配置下拉里的临时测试配置已补上（2026-10-20）。Run with Coverage（JaCoCo、Coverage 工具窗口、编辑器绿 / 黄 / 红条）已补上（2026-10-21，单元测试和仪器测试）。剩覆盖率和仪器测试的真机验证（Debug 测试已随第 5 项完成）。
9. ✅ Gradle 文件辅助（2026-10-15，FEATURES.md 第 20 节）：version catalog 跳转、悬停、Find Usages、`libs.` 补全，依赖和插件新版本警告 + Alt+Enter “Change to …”。Project Structure 对话框已补上（第 61 节），Alt+Enter 的 “改用 version catalog” 迁移修复也已完成（2026-10-20）。
10. ✅ Problems / TODO 工具窗口、Local History（2026-10-15，FEATURES.md 第 18 节）。
11. ✅ 代码生成：✅ Surround With（Ctrl+Alt+T，FEATURES.md 第 28 节，照 IntelliJ 的 Kotlin / Java surround descriptor）。✅ Live Templates（Tab 展开、变量间跳转与同步，FEATURES.md 第 29 节，模板取自 IntelliJ 的 Kotlin.xml / Java.xml）。✅ Generate（Alt+Insert：toString、equals / hashCode，Java 的构造函数、getter / setter，FEATURES.md 第 30 节，照 IntelliJ 默认模板）。✅ Override / Implement Methods（Ctrl+O / Ctrl+I，2026-10-16，FEATURES.md 第 36 节）。模板进补全列表已补上（2026-10-20，FEATURES.md 第 29 节）。自定义模板、关闭内置模板已补上（2026-10-20，Settings › Editor › Live Templates，存 `live-templates.json`，FEATURES.md 第 29 节）。

**P2：重，可以长期留给 AS**

12. Compose Preview / Layout Editor（依赖 layoutlib，成本极高，建议用投屏替代）。
13. Profiler、Layout Inspector、App Inspection。
14. APK Analyzer、SDK Manager、Lint 视图。（Generate Signed Bundle / APK 已做，2026-10-18，FEATURES.md 第 59 节：注入 `android.injected.signing.*`，项目没带 keystore 也能出签名包；Create new… 用 keytool 新建 keystore。Remember passwords 和 Destination Folder 已补上，2026-10-20。）

**内存实测（Linux 容器，软件渲染）**：Junction 打开一个 104 个 .kt 文件的 Android 项目约 241 MB；打开 junction 仓库（含 Cargo 依赖源码）首次建索引峰值 674 MB、约 20 秒，再次启动读缓存 396 MB、CPU 约 2 秒。作为对比，最小 Kotlin/JVM 项目的 Gradle Daemon 669 MB + Kotlin Compile Daemon 378 MB。结论：Rust 能省掉 IDE 自身的内存和启动扫描，但 Gradle Sync / 构建 / AS 级 Kotlin 语言服务本身是 JVM，省不掉，只能做到按需启动、少同步。

### 6.3 尚未在真机验证

- Windows：签名对话框的 Remember passwords（DPAPI）；Terminal（PowerShell / ConPTY）、Running Devices 和 Device Explorer 的完整流程、双击 Shift、大部分 2026-10-12 之后的功能。2026-10-17 之后只在 Windows 编译检查过、没在 Windows 上跑过的：直接运行 `gradlew.bat`（参数转义）、`SetErrorMode` 和缺 DLL 提示、`RtlGetVersion` 取系统版本（这一项维护者的构建输出里已经看到了）、keytool.exe 新建 keystore。
- macOS：钥匙串保存签名密码；.app 能否正常运行、macOS keymap、打包脚本、符号链接路径下的多仓库行为。
- 容器里测不了：跟随系统主题、GPG 密钥列表、原生文件选择框、外部修改后自动刷新。

## 7. 时间线（关键节点）

| 日期 | 内容 |
|---|---|
| 2026-10-06 | 技术选型；M1–M2 |
| 2026-10-07 | M3–M6、M7 索引、Diff/Merge 按 AS 结构重写 |
| 2026-10-08 | 改名 Junction / Junction Studio；M8 搜索；编辑器标签与分屏；GitHub Actions 发版 |
| 2026-10-09 | 全量回归 + 全部修复 + 复测；External Libraries |
| 2026-10-10 | 性能优化（Linux 内核仓库）；UI 层重构（拆 Workspace / log_view / dialogs）；全部换成 AS 图标；多语言高亮 |
| 2026-10-11 | 多仓库与 AS 逐项对齐；实现/覆盖 gutter、Ctrl+点击全覆盖；第二轮性能优化；README 中英文与截图 |
| 2026-10-12 | 菜单全量回归；Find / Terminal 工具窗口；树的选择行为；Device Manager、Running Devices、Device Explorer |
| 2026-10-13 | Diff 与编辑器行号栏菜单、Blame 列、书签、标签页菜单、Copy Path/Reference；Log 引用标签配色；日常 Android 开发缺口评估 |
| 2026-10-14 | 编辑器语言智能（补全、自动 import、语法错误、保存后编译）；Gradle Sync；Clean 架构分层 |
| 2026-10-15 | Build / Run、Logcat、Android 资源、测试、Debugger 主体、Gradle JDK、Rename 等重构 |
| 2026-10-16 | Debugger 细节（日志 / 异常 / 方法断点、字段监视点、Mute）；Extract Method、Introduce Parameter、Change Signature；Translations Editor、drawable 预览、Merged Manifest；控制台改成只读编辑器 |
| 2026-10-17 | Call / Type / Method Hierarchy、Recent Locations、Inline Variable / Function、Unwrap、Safe Delete、会话恢复；架构复查（`arch_tests`）；缺 DLL 诊断；CI 加 Windows 检查 |
| 2026-10-18 | Generate Signed Bundle / APK 和新建 keystore；Build 窗口列出 Gradle 的 “What went wrong”；控制台往上滚不再被拉回底部；改为分支 + PR 合入 master |
| 2026-10-19 | 控制台 ANSI 颜色和 1 MB 输出上限；Inline Property / Field（顺带修了 Inline Variable 把成员属性当局部变量的 bug） |
| 2026-10-20 | 构件额度占满导致发版失败；发布流程改为直接上传到草稿 Release；签名打包的 Remember passwords（系统密码库）和 Destination Folder；Live Templates 进补全列表；Build 窗口 Sync 标签页；HotSwap / Apply Code Changes；Logcat 分屏；Gradle user home / Offline work / Distribution 设置，修复 Gradle JDK 设置不保存 |

更细的历史看 `git log`，每个提交的标题都写明了用户可见的变化。

## 8. 接手建议

0. 行为拿不准时，去 IntelliJ IDEA / Android Studio 的开源代码里看它们怎么做（`git clone --filter=blob:none --sparse https://github.com/JetBrains/intellij-community`，按需 `git show HEAD:<路径>` 读单个文件）；Optimize Imports 的默认排序、Introduce Variable 的取名规则和提示文案都按那里核对过。

1. 先编译运行一次，打开一个 Android 项目和一个多仓库目录，对照 FEATURES.md 熟悉界面。
2. 6.2 的 P0、P1 大多已完成，剩下的缺口在各项的“剩 …”里，以及 FEATURES.md 里标 ⬜ 的行。维护者的工作方式：让 AI 持续迭代（“参考 IntelliJ / Android Studio 源码，有需要再找我”），用中文沟通；完成一项就推分支、CI 绿了合入 master（见第 3 节“提交流程”）。新功能按第 4 节的 Clean 架构分层写。
3. 动 `vendor/`、索引缓存结构、`release.yml` 之前，先读第 3、4 节对应的坑。
4. 维护者实际在用的工程是 mytv-android（Windows 11）。它的 release 签名配置指向仓库里没有的 `app/keystore.jks`，直接 assembleRelease 会失败；用 Build › Generate Signed Bundle / APK… 带自己的 keystore 就能出包。
