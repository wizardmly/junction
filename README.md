# Junction Studio

[English](README.en.md) | 简体中文

Junction Studio — Git & Code Navigator：桌面客户端，一比一复刻 Android Studio / IntelliJ 的 Git 功能，以及跨语言的代码索引和跳转（Kotlin、Java、C/C++、Objective-C、Rust、Swift、Dart、Go、V 等，含 JNI、Dart FFI、Rust extern C、Swift/ObjC 桥接）。
技术栈：Rust + [GPUI](https://www.gpui.rs) + [GPUI Kit / gpui-component](https://github.com/longbridge/gpui-kit)。
Git 操作和 IntelliJ 的 git4idea 一样，直接调用 `git` 命令行。

![Commit 工具窗口、Diff 和 Git Log](docs/screenshots/overview-dark.png)

## 下载

每次推送到 master，GitHub Actions 会自动编译三个平台的安装包，发布在 [Releases › latest](https://github.com/wizardmly/junction/releases/tag/latest)（链接固定不变）：

| 平台 | 文件 | 用法 |
|---|---|---|
| Windows x64 | `junction-windows-x64.zip` | 解压后运行 `Junction Studio/junction.exe` |
| macOS（Apple 芯片和 Intel 通用） | `junction-macos-universal.dmg` / `junction-macos-universal.app.zip` | 把 Junction Studio 拖进"应用程序" |
| Linux x64 | `Junction-x86_64.AppImage` / `junction-linux-x64.tar.gz` | `chmod +x` 后运行 AppImage，或解压 tar.gz |

各文件的校验值在 `SHA256SUMS.txt` 中。

macOS 版没有经过 Apple 公证，首次打开会提示"Apple 无法验证 Junction Studio.app 是否包含恶意软件"。可以在 系统设置 › 隐私与安全性 里点"仍要打开"，或者执行：

```sh
xattr -dr com.apple.quarantine "/Applications/Junction Studio.app"
```

也可以按下面的步骤在自己的 Mac 上编译，本机编译的应用不会出现这个提示。

## 功能

完整对照表和进度见 [docs/FEATURES.md](docs/FEATURES.md)。接手开发请先读 [HANDOFF.md](HANDOFF.md)（交接文档：现状、编译测试、代码结构、约定和待办）。

- **Git 工具窗口 › Log**：分支树（收藏、按 `/` 分组、搜索）、彩色提交图（长边折叠为箭头）、Branch / User / Date / Paths 过滤、提交详情与变更树、GPG/SSH 签名信息、完整的提交右键菜单（Cherry-Pick、Reset、Revert、Reword、Fixup、Squash、Drop、Interactively Rebase from Here、Push All up to Here…）。
- **Commit 工具窗口**：非模态提交、Changelist 或暂存区模式、按 chunk / 按行部分提交、Amend、提交信息拼写检查和历史、Shelf 与 Stash。
- **Diff 与 Merge**：双栏 / 单栏 Diff，右侧可直接编辑，`>>` 回滚单块；三方合并工具（魔棒合并简单冲突、逐块接受）；冲突对话框；图片对比。
- **分支与远程**：标题栏分支弹窗（Recent / Local / Remote / Tags，全部分支动作）、Fetch / Pull / Push / Update Project 对话框、交互式 Rebase、Merge / Rebase 对话框、Tag、补丁、Worktree、Submodule、多仓库根。
- **文件级**：Annotate with Git Blame、文件历史、选中内容历史、编辑器 gutter 变更标记（Rollback / Stage 单块）。
- **GitHub / GitLab**：Token 登录、Pull Requests 工具窗口（评论、审查、合并）、Share Project、Create Gist、Clone 时列出账号仓库。
- **代码索引与跳转**：tree-sitter 内置索引 + 各语言 LSP；Go to Declaration（Ctrl+B / Ctrl+点击）、Find Usages、实现 / 覆盖 gutter 图标；External Libraries（Gradle、Cargo、Go、Dart/Flutter、C/C++ 等依赖和 SDK 源码）。
- **搜索**：Search Everywhere（双击 Shift）、Find / Replace in Files、Find Action、Recent Files、File Structure、Go to Line。
- **编辑器与外观**：标签页与分屏、AS 配色的语法高亮、IntelliJ 新 UI 图标、亮 / 暗主题、IntelliJ 与 macOS 快捷键。

## 截图

截图来自用 Junction Studio 打开的开源项目 [Now in Android](https://github.com/android/nowinandroid)，以及 Junction 自己的源码（External Libraries）。

### Git Log

![Git Log 暗色](docs/screenshots/log-dark.png)

![Git Log 亮色](docs/screenshots/log-light.png)

提交右键菜单：

![提交右键菜单](docs/screenshots/log-menu-dark.png)

### Commit 工具窗口与 Diff

![Commit 与 Diff 亮色](docs/screenshots/overview-light.png)

双栏 Diff（右侧可编辑，复选框选择要提交的块）：

![双栏 Diff](docs/screenshots/diff-dark.png)

### 分支弹窗

![分支弹窗](docs/screenshots/branches-dark.png)

### 交互式 Rebase

![交互式 Rebase](docs/screenshots/rebase-light.png)

### 冲突与三方合并

![冲突对话框](docs/screenshots/conflicts-dark.png)

![三方合并工具](docs/screenshots/merge-dark.png)

### Annotate with Git Blame

![Git Blame](docs/screenshots/blame-dark.png)

### 编辑器、跳转与用法

带 AS 配色和实现 / 覆盖 gutter 图标的 Kotlin 编辑器：

![编辑器](docs/screenshots/editor-dark.png)

在声明上按 Ctrl+B / Ctrl+点击，列出用法：

![用法弹窗](docs/screenshots/usages-dark.png)

### External Libraries

依赖库和 Rust 标准库源码进入索引，跳转后只读打开：

![External Libraries](docs/screenshots/external-libraries-light.png)

### 搜索

![Search Everywhere](docs/screenshots/search-everywhere-dark.png)

![Find in Files](docs/screenshots/find-in-files-dark.png)

## 在 macOS 上编译 Junction Studio.app

1. 从 App Store 安装 **Xcode**，装好后打开一次并同意许可协议。编译需要 Xcode 自带的 Metal 着色器编译器，只装 Command Line Tools 不够。
2. 安装 Rust：

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. 下载代码，编译并安装到"应用程序"：

   ```sh
   git clone https://github.com/wizardmly/junction.git
   cd junction
   packaging/build-macos-app.sh --install
   ```

   不加 `--install` 时只生成 `target/Junction Studio.app`。第一次编译需要 10 到 15 分钟，之后会快很多。

以后更新：在 `junction` 目录执行 `git pull`，再运行一次 `packaging/build-macos-app.sh --install`。

## 运行

需要 Rust 1.90+ 和 `git`。

```sh
cargo run --release -- /path/to/repo     # 不传路径时打开当前目录
JUNCTION_THEME=light cargo run           # 亮色主题
```

- **Windows**：需要 Visual Studio Build Tools（MSVC）。Windows 11 上窗口使用 Mica 背景。
- **macOS**：需要 Xcode（见上文）。
- **Linux**：先安装依赖：`sudo apt install libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libvulkan1 libfontconfig-dev libssl-dev`。

跳转和 External Libraries 会使用本机已安装的语言服务器与 SDK（如 rust-analyzer、clangd、kotlin-lsp、gopls、Dart SDK），没有时退回内置索引。

## 结构

```
src/git/        git 后端：命令执行与 Console 记录、log 解析、提交图布局、refs、status/commit、merge、rebase
src/hosting/    GitHub / GitLab API
src/index/      代码索引：tree-sitter 符号、跨语言桥接、LSP、External Libraries、跳转与用法
src/model.rs    仓库状态（后台加载、选中提交、操作 + 通知）
src/ui/         界面：workspace（主窗口）、log_view、commit_view、diff_view、merge_view、对话框、搜索、编辑器
src/theme.rs    IntelliJ Int UI 亮 / 暗配色
vendor/         打过补丁的 gpui-component 等依赖（改动处标有 "Junction:"）
```

`cargo test` 运行解析器、提交图布局、索引和文件树等单元测试。
