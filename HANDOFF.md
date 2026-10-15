# Junction Studio 交接文档

写给接手这个仓库继续开发的人或 AI。读完这一份，再按需看 [docs/FEATURES.md](docs/FEATURES.md)（逐项功能状态）和 [docs/DIFF_MERGE.md](docs/DIFF_MERGE.md)（Diff/Merge 规格），就能接着做。

最后更新：2026-10-14，对应 master `1cc07ef` 之后。

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
- `cargo test` 当前约 196 个单元测试（另有若干 `#[ignore]` 的基准和探针）。
- 绝大多数功能**只在 Linux（Xvfb）下测过**。Windows 上验证过的：启动、任务栏点击最小化、旧索引缓存崩溃修复、Device Manager 的 Wi-Fi 自动连接。macOS 基本未在真机验证。

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
  workspace/        主窗口：tabs、navigation、search、files、panels、chrome、menu、vcs、events、history
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

现有的 `git/`、`index/`、`android/` 暂不迁移，新功能需要它们时在 `infra/` 里包一层适配器。行为不确定时查源码：编辑器、补全、Problems、Local History、测试、Debugger 看 intellij-community；Logcat、Build/Run、Gradle Sync、资源、设备相关看 AOSP `platform/tools/adt/idea`（都是 Apache 2.0）。

### 需要知道的约定和坑

- **UI 目录拆分方式**：`workspace/`、`log_view/`、`dialogs/` 等目录里，子文件各写一个 `impl X` 块，方法是 `pub(super)`，开头 `use super::*`。新功能按这个方式加到对应子文件。
- **不要在 UI 里直接拼 git 命令**：业务逻辑放到 `git/` 里写成可测试的函数（用 `TestRepo` 写测试），UI 通过 `model.run_task` / `run_operation` 在后台执行。不要在主线程同步跑 git。
- **单个函数不超过 300 行**：render 函数拆成 `render_*` 辅助方法（方法需要 `&mut Context` 又和其他 `cx` 用法并列时返回 `AnyElement`）。
- **缓存视图**：面板是 GPUI 的 cached view（`ui::common::cached`），`Workspace::redraw_panels` 在 model / index 变化时通知它们。**一个缓存面板如果 render 里读了新状态，必须 observe 它并 `cx.notify()`**，否则界面不刷新。
- **索引缓存兼容**：`ProjectIndex` / `LibraryCache` 的**第一个字段必须是 `version: u32`**，一律通过 `index::store::read_cache` 读（先校验版本，再按文件大小限制 bincode 解码）。**改了缓存结构就把 `CACHE_VERSION` 加一**。曾经因为旧缓存被新结构解码，Windows 启动时申请了几 EB 内存直接崩溃。
- 缓存位置：项目索引 `<gitdir>/junction/index.bin`；外部库索引 `~/.cache/Junction/libraries`（跨项目共享）；jar 源码解压到 `~/.cache/Junction/sources`。外部库文件以绝对路径作为键，只读打开。
- **vendor 补丁**：`vendor/` 下的改动都标有 `Junction:` 注释（对话框居中不滑入、通知右下角堆叠且最旧在下、通知 10 秒自动消失、分层彩色图标、按钮尺寸和悬停色、补全插入 import 和括号内光标、Ctrl+Space 手动补全 `trigger_completion`、导出 Diagnostic 等）。升级 gpui-kit 时要从 registry 重新拷贝这些 crate，再把 `Junction:` 的改动补回去。
- **图标**：全部用 IntelliJ 官方新 UI SVG（Apache 2.0，见 `assets/as-icons/NOTICE.md`）。源文件放 `assets/as-icons/src/<名字>.svg` 和 `<名字>_dark.svg`（从 `JetBrains/intellij-community` 的 `platform/icons/src/expui/...` 取），然后 `python3 tools/as_icons.py` 重新生成 `assets/as-icons/gen` 和 `src/ui/as_icons_gen.rs`。代码里 `use crate::ui::as_icons as icons;`，直接传 `icons::X`。不要再引入别的图标库。
- **语法高亮**：`src/ui/file_editor.rs` 的 `language_for(path)` 映射扩展名，`grammars()` 注册自己的查询覆盖工具包自带的（工具包里 Swift、CMake、proto、C# 等的查询是空的）。**`.v` 是 V 语言，不是 Verilog**。测试 `every_language_highlights` 检查每种语言的关键字颜色，加语言时要加样例。
- **多仓库**：`model.rs` 里每个根一个 `RootState`；`route(path)` / `run_in_roots` 按文件所在的根执行 git；Log 合并各根历史（`git::log::merge_logs`）。判断提交属于哪个根要用 `rev-parse <hash>^{commit}`，因为 `rev-parse --verify <40位hex>` 对任意 40 位哈希都返回成功。macOS 上 `git rev-parse --show-toplevel` 返回真实路径，而 `RootState.path` 是拼出来的，有符号链接时路径比较会失败（疑似导致 mac 上切换仓库后 Project 树不刷新，未确认）。
- **Running Devices 输入**：非 ASCII 文字（中文等）由 `assets/text-input` 里的小程序以 shell 身份通过 UiAutomation 直接写进焦点输入框；失败时退回 Gboard Unicode 组合输入，再不行就丢弃并通知。**绝对不要用剪贴板粘贴**（会弹系统提示，明确不接受）。手机上如果有其他程序占着 UiAutomation（例如 Maestro、其他 instrumentation），小程序起不来，通知里会列出占用者。
- **设置和日志**：Linux `~/.config/Junction/settings.conf`；Windows `%APPDATA%\Junction\`，启动崩溃看 `stderr.log`（内存分配失败、panic），`crash.log` 只记 SEH 异常。
- 编译警告基线：约 9 个已有的 dead-code 警告，改动不要新增警告。

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
| 多仓库：子仓库文件的部分操作 | Create Patch、合并冲突工具、Project 窗口右键 Git 菜单对嵌套仓库里的文件仍在当前选中的仓库执行；多仓 Apply Patch 的 changelist 选择 |
| Log 里 AS 的合并引用标签（“origin & main” 叠放） | 未做 |
| 编辑器多行标签、双击标签最大化编辑器 | 未做 |
| 同一文件在两个分屏里共享一份编辑缓冲 | 目前两边各一份 |
| 菜单里置灰的 STUB 项 | Project 视图的 Find Usages、Analyze、Move/Copy（F6/F5）、Reformat/Optimize Imports、Mark Directory As、Module Settings、Override File Type、Local History |
| jar 没有源码时的类文件骨架 + “Download Sources” 横幅 | 未做（已列为下一步候选） |
| 实现/覆盖标记的继承关系 | 只按简单名匹配，没有解析 import |
| 不打算做 | Project 窗口的 Edit Scopes、Group Tabs、View Mode、Move to、Resize、Remove from Sidebar，Android / Packages 视图（Junction 没有对应的 IDE 布局和 Android 模块模型） |

### 6.2 用 Junction 替代 AS 做日常 Android 开发的缺口（2026-10-13 评估）

现在能替代 AS 的：Git、看代码和跳转、全局搜索、终端、Device Manager、投屏、Device Explorer。还不能替代的核心是下面 P0 四项，缺了每天都得回 AS。

**P0：不做就无法日常开发**

| # | 功能 | 现状 | 实现思路 |
|---|---|---|---|
| 1 | 编辑器语言智能 | ✅ 2026-10-14 按“A+B”方案完成：索引补全 + 自动 import + 语法错误 + 保存后编译报错（FEATURES.md 第 14 节）。不常驻 Kotlin 语言服务（省 1–2 GB）；kotlin-lsp 以后可做成设置里的可选开关 | 剩 Alt+Enter 快速修复、Reformat、参数提示 |
| 2 | Gradle Sync | ✅ 2026-10-14 轻量版完成：init script 解析各模块 classpath 并下载缺的依赖和源码，外部库按结果精确列出；build 文件变化时自动同步，打开项目只提示不同步（FEATURES.md 第 15 节） | 剩 Build 窗口 Sync 标签页、Build Variants（随第 3 项） |
| 3 | Build + Run | ✅ 2026-10-15 完成：标题栏 Make / 设备 / 运行配置 / Run / Stop，Build、Run、Build Variants 工具窗口（FEATURES.md 第 16 节） | 剩 Debug、Apply Changes、Edit Configurations 对话框、真机验证 |
| 4 | Logcat | ✅ 2026-10-15 完成：Logcat 工具窗口（Alt+6），设备下拉、AS 过滤语法、AS 配色、PROCESS STARTED / ENDED、堆栈跳源码、Standard / Compact、暂停、清空、重启（FEATURES.md 第 17 节） | 剩过滤输入补全和历史、多个 Logcat 标签页、截屏录屏按钮、真机验证 |

**P1：每天会用，可暂时回 AS**

5. Debugger（断点、单步、变量、Evaluate、Attach）：Device Explorer 已经走 JDWP 列进程，有底层基础，UI 工作量最大。
6. 符号级重构（跨文件 Rename、Extract、Change Signature、Optimize Imports）：走 LSP rename / codeAction。
7. ✅ Android 资源（2026-10-15，FEATURES.md 第 19 节）：`R.x.y` / `@x/y` 跳转、悬停显示值、Find Usages、颜色和图片 gutter 预览。剩 Translations Editor、Manifest 合并视图、vector drawable 预览。
8. 测试：gutter ▶ 跑单个测试、结果树；Gradle test 任务 + 解析 JUnit XML。
9. ✅ Gradle 文件辅助（2026-10-15，FEATURES.md 第 20 节）：version catalog 跳转、悬停、Find Usages、`libs.` 补全，依赖和插件新版本警告 + Alt+Enter “Change to …”。剩 Project Structure 对话框、“改用 version catalog” 之类的迁移修复。
10. ✅ Problems / TODO 工具窗口、Local History（2026-10-15，FEATURES.md 第 18 节）。
11. 代码生成：Alt+Insert、Live Templates、Surround With。

**P2：重，可以长期留给 AS**

12. Compose Preview / Layout Editor（依赖 layoutlib，成本极高，建议用投屏替代）。
13. Profiler、Layout Inspector、App Inspection。
14. Generate Signed APK/AAB、APK Analyzer、SDK Manager、Lint 视图。

**内存实测（Linux 容器，软件渲染）**：Junction 打开一个 104 个 .kt 文件的 Android 项目约 241 MB；打开 junction 仓库（含 Cargo 依赖源码）首次建索引峰值 674 MB、约 20 秒，再次启动读缓存 396 MB、CPU 约 2 秒。作为对比，最小 Kotlin/JVM 项目的 Gradle Daemon 669 MB + Kotlin Compile Daemon 378 MB。结论：Rust 能省掉 IDE 自身的内存和启动扫描，但 Gradle Sync / 构建 / AS 级 Kotlin 语言服务本身是 JVM，省不掉，只能做到按需启动、少同步。

### 6.3 尚未在真机验证

- Windows：Terminal（PowerShell / ConPTY）、Running Devices 和 Device Explorer 的完整流程、双击 Shift、大部分 2026-10-12 之后的功能。
- macOS：.app 能否正常运行、macOS keymap、打包脚本、符号链接路径下的多仓库行为。
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

更细的历史看 `git log`，每个提交的标题都写明了用户可见的变化。

## 8. 接手建议

1. 先编译运行一次，打开一个 Android 项目和一个多仓库目录，对照 FEATURES.md 熟悉界面。
2. 下一步按维护者定的顺序做 6.2：测试 → Debugger。新功能按第 4 节的 Clean 架构分层写。
3. 动 `vendor/`、索引缓存结构、`release.yml` 之前，先读第 4 节对应的坑。
