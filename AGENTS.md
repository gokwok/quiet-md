# 开发约定（给 AI 助手和人）

先读 README.md 和 docs/ARCHITECTURE.md；涉及 iOS 任务时再读 prototypes/ios/README.md。AGENTS.md 与 CLAUDE.md 内容相同，修改时两份一起改。

## 产品原则

- 代码、内置指南、文档和资源都只在这一份仓库里维护。界面细节以代码里的 `Theme` token 和现有实现为准，不再另设设计参考文档；和代码对不上时，以代码为准。
- 保持现有元素、顺序、行为、快捷键和文案；视觉使用现有 Theme 语义 token。
- 桌面端唯一的结构性偏离：侧栏沉在窗口底色上（无面板、无边框、无阴影），笔记是贴齐窗口上/右/下边的白色平面（无 gutter、圆角、边框、阴影）。
- 手机端（宽度 `< 700pt`）按手机交互习惯设计而非缩小桌面，采用单栏导航、底部卡片面板和 iOS 26+ 原生 Liquid Glass，细节见 `prototypes/ios/README.md`。

## 代码

- **不跑 rustfmt。** 代码是刻意的紧凑风格，新代码照周围代码的风格写，注释密度也一样。
- **文档引擎在 `crates/core`（`fdl_core`），不依赖 GPUI。** 模型、格式、编辑内核、文档库、导入和 Markdown 导出放这里，只能引用 core 里的模块；界面代码放根 crate。见 docs/ARCHITECTURE.md。
- **AI 核心在 `crates/ai`（`fdl_ai`），只依赖 `fdl_core`，不依赖 GPUI。** 负责提示词拼装（如酒馆核心 `tavern` 的预设、宏、正则）与模型输出成形；AI 生成内容写入文档同样必须经过 `editor::Core`（如 `Core::insert_generated`）。
- **改文档结构只经过 `editor::Core`。** 顺序是：`checkpoint` → Core 方法 → `Editor::changed()`。新增结构性操作时，同时在 `fuzz_tests.rs` 的 `step` 里加一种随机操作。
- 能有子级的只有 `BlockKind::nests()` 的 block 和容器（引用、高亮块、分栏、列）。代码块里没有样式和行内对象。
- **DocxXML 写出的格式要稳定。** 改了读写逻辑，`crates/core/tests/fixtures/all-blocks.xml` 必须仍能原样往返。新增 block 类型时，把它的规范写法加进这个 fixture。
- **颜色用 `Theme::*` 语义 token**，其次是 `theme::craft` 色阶（`c(craft::N300)`）。只有实测值才写十六进制并注明出处。阴影用 `theme::shadow_*`，菜单外壳用 `ui::popover` / `ui::menu_row`。
- **新图标先找 Nucleo（UI Outline 18，和现有图标同一套）**，Nucleo 里没有合适的再自己画。自己画的在 SVG 开头注释写明「本项目绘制」。增删图标时同步 THIRD_PARTY.md 里的数量和清单：Nucleo 授权允许开源项目最多用 100 个。
- 改了 `vendor/` 的代码，同步更新 vendor/PATCHES.md。
- 改了文档库目录布局、会让旧版本误读时，把 `store::FORMAT` 加一。Meta 的新字段一律 `#[serde(default)]`。

## iOS（`prototypes/ios/`）

- **独立依赖与拼装构建。** iOS 不在根 Cargo workspace 里，使用自己的 `prototypes/ios/Cargo.toml` 和 `Cargo.lock`（锁定 `gpui-pre` / `gpui-pre-platform` / `gpui-pre-ios` 0.3.12），通过路径依赖共用 `fdl_core` 和 `fdl_ai`。Rust 工具链锁定在 `rust-toolchain.toml`（`1.99.1`）。构建时由 `stage-source.sh` 把桌面 `src/`、`build.rs` 拷到 `target/ios/stage` 并应用 `port.patch`，再合入 `prototypes/ios/` 源码与资源。
- **不手改 `port.patch`。** 改动桌面侧挂接点时，用 `prototypes/ios/dev-tree.sh` 生成 `target/ios/dev`，在里面修改后再运行 `prototypes/ios/make-patch.sh` 重新生成补丁。新 iOS 代码尽量写在 `prototypes/ios/*.rs`，补丁里只留挂接点。改了桌面 `src/` 后注意确认 `port.patch` 仍能干净应用。
- **原生 UIKit / Liquid Glass 叠层。** iOS 上 GPUI 的 Metal 画面不透明，iOS 26+ 的 Liquid Glass（`glass.rs`）、系统菜单（`menu.rs`）、编辑浮层（`EditorSurface.m` / `editor_surface.rs`）、退场快照（`shot.rs`）和面容 ID 遮罩（`lock.rs`）均为盖在 Metal 视图上的原生 UIKit 视图。iOS 侧 Objective-C 桥接用 `objc2` / `block2`（区别于 macOS 侧的 `objc` / `block`）。
- **品牌、专属资源与开源许可。** iOS 主屏显示名为 **Margin**（¶ 图标）。iOS 专属静态资源（如 `avatars/`、`icons/`、`washi/`）放 `prototypes/ios/` 下，不要放进根 `assets/` 以免打进桌面包。改动 `prototypes/ios/Cargo.lock` 后运行 `prototypes/ios/licences.sh` 重新生成 `licences.txt`。

## 提交前

```sh
scripts/check.sh        # 桌面 workspace：cargo test --release + 指南校验 + clippy --all-targets 零警告
```

`scripts/check.sh` 不编译 iOS。改了 `prototypes/ios/`、可能影响 `port.patch` 的桌面 `src/`，或 `fdl_core` / `fdl_ai` 公共接口时，额外检查 iOS：

```sh
prototypes/ios/run-simulator.sh            # 不带参数：打补丁并构建签名模拟器包
prototypes/ios/verify-simulator.sh "$dev"  # 专用 iPhone 18 Pro 模拟器 UI 冒烟测试
```

改了编辑内核时，多跑一些随机种子：`FDL_FUZZ_SEEDS=2000 cargo test --release random_edits`。

只移动代码、不改外观的重构，用 `tools/harness/compare.py` 对比改动前后的截图，必须完全一致（见 tools/harness/README.md）。

想每次提交自动检查：`git config core.hooksPath scripts/hooks`。

发布桌面版本时，把根目录 `Cargo.toml` 和 `crates/core/Cargo.toml` 的版本号一起更新（`crates/ai` 和 `prototypes/ios` 均为 `publish = false`）。未经用户明确要求，不创建提交。

## 默认发布流程

用户提及「发布」时，默认通过 TestFlight 分发 iOS 内部测试版。用户明确指定桌面发布、App Store 上架或其他渠道时，以指定渠道为准。

- 复用 App Store Connect 的「留白 Quiet.md」（`6836195254`）、Bundle ID `com.s1dashu.quietmd.prototype` 和内部组「个人设备测试」。
- 按 [iOS 发布说明](prototypes/ios/README.md#通过-testflight-发布到-iphone)核对下一个构建号，运行仓库检查和模拟器冒烟测试，用 `prototypes/ios/testflight/archive.sh` 生成带匹配 dSYM 的真机归档，再用同目录的 `upload.sh` 上传。每次发布保留归档。
- 跟进 Apple 处理及出口合规问题，核实构建已进入 **Testing**、目标组已关联且本人已受邀或已接受邀请，再报告可以安装。登录或双重验证交由用户在 Apple 界面完成。

## 用户数据（绝对不要碰）

- 产品名 Quiet.md，二进制 `quiet_md`，默认文档库 `~/Documents/Quiet.md`。那是使用者的真实数据：不要提交进仓库，不要删改里面的文件。别的应用的进程和数据一律不碰，结束进程只能匹配本仓库自己编译出来的二进制。
- 本地预览版是 `dist-preview/quiet.md.app`，用 `scripts/preview.sh` 启动或更新，用的也是默认文档库。应用首次启动时，由 `src/guides.rs` 在空文档库中初始化编入二进制的「开始指南」，不覆盖已有文档库。
- `shared-mock/library` 由 `python3 shared-mock/guides/build.py <空目录>` 从 `shared-mock/guides/src/*.xml` 生成：改了指南就重新生成，并用 `FDL_ROUNDTRIP_DIR=<目录>/docs cargo test --release library_round_trips -- --ignored` 确认能原样往返。指南只写已经实现的功能，标题不带 emoji 图标。
- 指南里的截图用 `shared-mock/guides/shots/shots.sh` 重拍（窗口会在屏幕中间浮一分钟左右，不抢焦点）；改了指南里展示的界面后记得重拍。
- 测试用的文档库一律放在临时目录，用 `QUIET_MD_HOME` 指定。

## UI 脚本（tools/harness）

规则见 tools/harness/README.md，要点：

- 不用真实系统剪贴板，`key cmd-v` 只在 `clip` 之后用。
- 不用 `activate` 抢焦点。
- 不点这些：
  - 设置 › 文档库的「在访达中显示」；
  - 链接和「网页搜索」；
  - 打印；
  - 快速查看。
- 不用 `open` 命令。
- 选文件用 `pick`。

## 环境注意

- shell 是 zsh：没有匹配的通配符会直接报错，`--include=*.rs` 这类参数要加引号。
- `grep` 实际是 ugrep。
- macOS 的 `sed -i` 要写成 `sed -i ''`。
- 没有 `timeout` 命令。
