# Quiet.md iOS 原型

原型锁定 `gpui-pre`、`gpui-pre-platform` 和 `gpui-pre-ios` 的 0.3.12。构建脚本把当前桌面源码复制到 `target/ios/stage`，再应用 [GPUI 适配补丁](port.patch)，直接编译整个桌面应用：`Workspace`、文档库、主页、侧栏、搜索、回收站、设置和编辑器都是同一份代码，并通过路径依赖共用 `fdl_core`。桌面 Cargo workspace 继续使用自己的依赖清单。

iOS 自己的代码在本目录，补丁里只留挂接点：

| 文件 | 内容 |
|---|---|
| `app.rs` | 入口（`crate::ios`）；手机版式的判断、安全区、抽屉、长按当右键、左缘返回、文件选择器的占位提示 |
| `keybar.rs` | 键盘上方的工具条 |
| `touch.rs` | 剪切 / 复制 / 粘贴 / 全选 的浮动菜单（作为 `editor` 的子模块编译） |
| `port.patch` | 上游 GPUI 的 API 差异，加上各处的手机版式分支 |

## 手机上的界面

宽度小于 700pt 时是单栏版式。更宽时走桌面的两栏版式，但 `Info.plist` 还没有声明 iPad，这条分支没有在设备上跑过：

- **按手机的习惯，不是缩小的桌面。** 主页标题下是一整条搜索，新建是右下角的悬浮按钮；文档页顶栏只有返回、目录和「…」。外壳的菜单（长按卡片或行、文档的「…」）从底部升起成动作面板，占满屏宽，下面压暗。菜单行和侧栏行 44pt，按钮 42pt，字号 16pt。手指抬起后不留悬停高亮。
- **编辑器的块菜单和添加面板也是底部面板。** 块菜单占满屏宽，类型图标六个一行、每格 44pt；「缩进和对齐」「颜色」点开后换下主面板，顶上一行返回；添加面板贴在键盘工具条上方。
- **选区工具栏停在键盘上方。** 点编辑菜单的「格式」后，工具栏占满屏宽停在底部，按钮 40pt，可横向滑动；类型、颜色、更多的下拉是它上方的整宽面板，色块 40pt。
- **从主页开始。** 侧栏是从左侧滑出的抽屉，点遮罩或选了目标就收起。左缘右滑：文档、文件夹和回收站回主页，设置返回，主页拉出抽屉。
- **安全区。** 顶栏在状态栏之下，内容避开 Home 指示条；键盘弹出时正文和浮层都收在键盘上方。状态栏的字跟着应用的深浅色走。
- **键盘工具条。** 撤销、重做、添加块、块菜单、标题、列表、缩进、粗体等，只派发编辑器已有的动作。右端的按钮收起键盘。
- **编辑菜单。** 长按选词（可继续拖动）、双击选词，或在光标上再点一下，浮出 剪切 / 复制 / 粘贴 / 全选；「格式」换成原有的选区工具栏。阅读模式下长按复制的代码已接上，尚未在模拟器里验证。
- **长按当右键。** 侧栏的行、主页的卡片、回收站的行，原本挂在右键上的菜单都由长按打开。
- **目录** 的按钮在顶栏，列表是临时浮层，选了标题就收起，不记进设置。
- 搜索、「移动到」、文档信息等对话框按屏幕宽度重排；回收站在窄屏不显示「位置」一列。页边距在窄屏是 20pt。

文档库在应用沙盒的 `Documents/Quiet.md`，首次启动放入「开始指南」。编辑后按桌面的节奏保存，进入后台时立即保存。

### 还没有的

- **选文件、存文件。** `gpui-pre-ios` 0.3.12 没有实现系统的文件选择器，所以导入、导出 / 下载、从文件插入图片和附件、上传封面都还不能用，点了会如实提示。入口都经过 `app.rs` 的 `prompt_for_paths` / `prompt_for_new_path`，接上原生选择器时只改这两个函数。
- 多窗口、快速笔记、打印、自动更新在 iOS 上没有入口。
- 「开始指南」里那张四列的表格，第二、三列之间有一道约 10pt 的缝，原因还没查。
- 依赖悬停或拖动的控件还没有触屏替代：表格的行列手柄、图片缩放、侧栏和主页的拖动排序、块的拖动。
- 表格的尺寸选择器、表格工具栏、链接和公式的编辑面板仍是桌面尺寸的浮层；打字触发的「/」菜单跟着光标，没有改成底部面板。主页的标签行、设置页的控件仍是桌面尺寸。
- 抽屉不能左滑收起；面板和页面切换没有过渡动画。
- 公式、链接、代码语言等带输入框的编辑器浮层，没有逐个在键盘弹出时检查过位置。

## 改代码

`port.patch` 不手改。先建一份可编辑的源码树，在里面改，再重新生成补丁：

```sh
prototypes/ios/dev-tree.sh      # target/ios/dev = 当前桌面源码 + port.patch
# 编辑 target/ios/dev/src/…；新的 iOS 代码尽量写在本目录的 .rs 里
prototypes/ios/make-patch.sh    # 由 target/ios/dev 重新生成 port.patch
prototypes/ios/run-simulator.sh "$device"
```

桌面的 `src/` 变了之后，补丁可能打不上：删掉 `target/ios/dev` 重新建，按报错的位置修。

`Cargo.toml` 关掉了 `gpui-pre` 的调试断言：桌面代码会对同一元素再次设置 hover / active 样式（后设的生效），上游只在调试构建里对此 panic。

## 在 Apple Silicon 模拟器运行

需要 Xcode、已安装的 iOS Simulator 运行时，以及仓库指定的 Rust 工具链。

```sh
rustup target add --toolchain 1.99.1 aarch64-apple-ios-sim
device=$(xcrun simctl create 'Quiet.md Prototype' \
  com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro \
  com.apple.CoreSimulator.SimRuntime.iOS-27-0)
xcrun simctl boot "$device"
xcrun simctl bootstatus "$device" -b
prototypes/ios/run-simulator.sh "$device"
```

省略设备参数时，脚本只构建并签名 `target/ios/Quiet.md Prototype.app`。原型使用独立的 bundle ID，文档库在模拟器的沙盒里。测试时用 `SIMCTL_CHILD_QUIET_MD_HOME` 指定别的文档库；相对路径落在沙盒的临时目录。

## 验证

安装 XcodeGen 后，在上面的专用 iPhone 18 Pro 模拟器运行以下命令。测试按该型号的屏幕坐标操作，要求软件键盘可见。

```sh
prototypes/ios/verify-simulator.sh "$device"
```

测试用 `QUIET_MD_IOS_SAMPLE` 让文档库只放 [sample.xml](sample.xml) 并直接打开它，在独立的临时文档库里长按拖动、双击选中 `Quiet`、点击拼音键盘输入 `nihao`，再选择「你好」。`QUIET_MD_IOS_VERIFY_BACKGROUND` 关掉定时保存，测试按 Home 后由脚本核对磁盘中的文字、合并单元格和 LaTeX 源码，然后重启并截图。可在生成的 `Result.xcresult` 查看触屏选区、拼音候选和提交后的截图。
