# 存储格式

## 文档库目录

默认位置：`~/Documents/Quiet.md`（「文稿」里的独立文件夹，用户能直接看到和备份）。可以用环境变量 `QUIET_MD_HOME` 指定别处。

```
<空间>/
├── space.json           {id, name, icon, created}；id 是持久 UUID，跟随空间移动
├── format.json          {"format": 4}：目录布局版本（store::FORMAT）
├── settings.json        窗口级 UI 设置（侧栏宽度等）
├── docs/<uuid>.json     一篇文档或一个文件夹的 Meta，以及 preview 列表缓存
├── docs/<uuid>.xml      DocxXML 正文；Markdown 文档用同名 .md
├── assets/              图片、附件；新插入资源写相对路径 assets/…
├── backdrops/           这个空间的背景
└── trash/
    ├── <uuid>.json      {meta, deleted, root, preview}
    └── <uuid>.xml       或 .md；文件夹只有 JSON
```

本机应用数据目录（macOS 为 `~/Library/Application Support/quiet-md`）的 `spaces/<空间 id>/` 保存 `documents.json` 和 `.lock`。`documents.json` 保存 `opened`、`catalogue_closed`、`reading`；外部本机文件的整条记录（包括绝对路径 `path`）也只在这里。删除进回收站后保留本机阅读状态，恢复时复用。测试可用 `QUIET_MD_LOCAL_DATA` 指定临时应用数据目录，不能指向同步空间内部。

应用的家仍由 `Store::home_dir()` 决定，保存本机 `spaces.json`、偏好、快捷键、主题和表情记录；文档资源由当前 `Store::root_dir()` 决定。

### Meta 与列表缓存

| 字段 | 含义 | 省略时 |
|---|---|---|
| `created` / `modified` | unix 秒 | 必填 |
| `pinned` | 置顶 | false |
| `starred` | 收藏时间，0 表示未收藏 | 0 |
| `folder` | 文件夹名称；文件夹只有 JSON，不对应正文 | 普通文档 |
| `parent` | 父文件夹 id | 顶层 |
| `order` / `root_order` | 文件夹内 / 顶层顺序 | 未手动排序 |
| `width` | `standard` / `wide` / `full` | standard |
| `info` | 文档信息行：`{hidden, created, modified}` | 只显示修改时间 |
| `refs_hidden` | 隐藏引用列表 | false |
| `markdown` / `name` | Markdown 正文及其标题 | XML 文档 |
| `field_modified` | 每个同步字段的独立修改时间（毫秒）；字段恢复默认值后仍保留时间 | 老记录按 `modified` 回退 |
| `conflict_source` | 冲突副本的稳定来源标识，用于中断重试去重 | 普通文档 |
| `preview` | 标题、图标、纯文本（列表摘要与搜索）、提醒和文档链接 | 空缓存 |

Meta 的新字段一律带 `#[serde(default)]`。只更新变动的文档 JSON，不因打开一篇文档而重写同步元数据；对同一记录的修改保留未改字段和未知字段。`list()` 读 JSON，不为库内文档读取正文。正文变更事件通过 `refresh_external` 刷新缓存，不改变打开编辑器时的冲突检测基线。

只识别小写、连字符形式的纯 UUID 文件名。`<uuid> 2.xml` 等不会直接成为列表条目，留给冲突处理；原文件不会被删除。正文暂时不存在时仍可显示缓存，打开失败不会将其替换为空文档。

### FORMAT 4 迁移与恢复

所有空间统一升级；旧版通过 `format.json` 拒绝打开。迁移在本机锁和旧版写锁内进行，先写独立元数据和本机状态，保留旧索引字节，最后写 FORMAT 4 标记。标记发布前失败可重试，旧索引保持可用；不改正文、附件字节。FORMAT 1 的父文档先按原有逻辑转成文件夹。

旧 `index.json` 备份在本机 `spaces/<id>/migration-format4/docs/index.before-format4.json`，回收站索引在同目录的 `trash/` 下；重复迁移遇到不同内容会额外保留一份，绝不覆盖旧备份。备份放在本机目录，避免把其中的绝对路径和阅读状态重新同步出去。

打开时把根目录、`docs/`、`trash/` 中遗留的 `*.tmp`、`*.corrupt-*` 和旧索引、备份、锁文件移出同步目录，保存在本机 `recovered-files/`，不直接丢弃潜在的恢复内容。FORMAT 4 元数据损坏时拒绝写入，不用默认值覆盖坏文件。

Apple 平台的读取、写入、移动、复制和删除经过 `NSFileCoordinator`；保存正文时基线检查和写入共享一次访问授权。其他平台仍使用 `std::fs`。

每个文件使用同目录唯一临时文件、`sync_all`、原子重命名；Unix 上再同步父目录。多文件操作先写目标元数据和内容，再清理来源；它不是跨文件的原子事务，中断时可能保留重复或待恢复条目，但不能删掉两边的正文。

### 冲突副本与版本清理

正文冲突保留当前文件；每个未解决版本创建独立 UUID 的「（冲突副本）」，继承原文档所在文件夹。XML 仅插入标题后缀，正文原始字节不变；Markdown 正文和 front matter 不改写。`<uuid> 2.xml/.md` 按同样方式处理，成功保存后才删除这个冲突文件。稳定来源标识和派生 UUID 让重试复用同一副本，已在回收站的副本也不重复创建。

元数据按 `field_modified` 逐字段取后改值；删除默认字段也留下时间。时间相同用规范 JSON 值确定结果，两个方向合并可收敛。排序值现在是顺序号（拖动时重排为 1、2、3），不能充当修改时间。JSON 写出时递归排序键，避免依赖的 `preserve_order` 特性影响合并结果。

所有副本 / 元数据合并落盘后，才把对应 `NSFileVersion` 标为已解决，再调用 `removeOtherVersionsOfItem` 清理版本。清理失败时保留副本并提示，后续从系统的版本状态中发现已解决冲突再清理；不维护额外的持久同步队列或重试日志。新到达但尚未保留的版本不会被顺带清掉。尚未做块级合并。

### 文件夹与排序

FORMAT 2 将旧父文档结构转换为同名文件夹，原父文档保留为文件夹内第一篇文档，其余子项保持旧顺序。旧迁移备份在升级 FORMAT 4 时移入本机 `recovered-files/`。

侧栏和主页手动排序共用 `Meta::place()`：未手动定位的条目按修改时间倒序在前，已定位的条目按顺序号排列，最后用 id 打破并列。首次初始化手动排序固定现有顺序。

## DocxXML

DocxXML 是飞书 / Lark 的文档 XML 格式。`crates/core/tests/fixtures/all-blocks.xml` 收录了每种 block 的规范写法，读入再写出必须逐字节不变（测试 `fixture_round_trips`）。

### 写出规则

- 顶层 block 每个占一行。子级换行，每层缩进两个空格。
- **表格单元格里的 block 写在一行内，不缩进。** 内容里的换行写成 `&#10;`（比如代码块的行），以免和排版用的换行混淆。
- 空的列表项如果有子段落，写成 `<li><p></p>…`。不加这个空 `<p>`，读回时子段落会被并进列表项。
- 折叠写成 `folded="true"`。图标和封面写在 `<title icon=… cover=… cover-x=… cover-y=…>` 上。

### 读入规则

- 只丢掉**源码排版产生的空白**：一个换行加缩进，以及嵌套 block 前面的空格和 Tab。其余空白都是内容，例如 `<p> </p>` 保留那个空格。
- 不认识的元素原样保留，写出时原样输出：block 级的是 `BlockKind::Raw`。
- 行内对象（公式、文件、按钮、日期提醒、@文档、#标签，以及不认识的行内元素）在文本里各占一个 U+FFFC，样式里存一个 `Inline { kind, xml }`：
  - `xml` 是读入时的原文，写出时原样输出，所以不认识的属性也不会丢；
  - `kind` 是读入时解析出来的类型，视图和导出只看它。
  - 编辑器新建或修改对象时，经 `docxml::to_inline` 先拼出规范 XML 再读回一遍，两者始终一致，存盘再读回也不变。

### 本地扩展

DocxXML 里没有、本应用自己加的写法。别的工具不认识这些写法，按上一节的规则它们只会被当作未知元素或属性：

- `<countdown duration start-time notify color timing/>`：倒计时。云端的倒计时是小组件，不在 DocxXML 里。
- `<page>标题<p>…</p></page>`、`<page view="card">…</page>`：页面 / 卡片（`BlockKind::Page`）。和 `<p>` 一样先是标题文字，后面是这一页的 block；子页面就存在这篇文档里，不是文档库里的另一篇。读入时页面总是折叠（内容只在打开这一页时显示），文件里不写 `folded`。导出 Markdown 时标题加粗，内容缩进在下面。
- `<cite type="tag">#名字</cite>`：#标签（`InlineKind::Tag`，名字不含 `#`）。文字内容就是看到的 `#名字`，导出和不认识它的版本显示的也是这几个字；名字只能是字母、数字、`_`、`-`，不合规的当未知行内元素原样留着。
- `<time … is-whole-day="true">`：全天提醒。模型里有 `isWholeDay`，但 XML 里没有这个属性。没有此属性、且时间是本地 23:59 的提醒，也按全天处理。
- 图片和附件的本地文件写成 `path="…"`（云端用 `token` / `href`）。`<figure><source path=… name=…/></figure>` 表示视频或文件。

### 已知限制

- **单元格内的尾随空格。** 如果单元格里某个 block 有嵌套子级，它文本末尾的空格或 Tab 读回时会丢失。原因：旧版本写单元格时带缩进，读入要能兼容，所以必须丢掉嵌套 block 前的空白。fuzzer 的 `round_trip` 对这一点做了相同的预处理（`trim_before_nested`）。
- 代码块、分割线、图片等不能有子级的 block 如果在内存里带了子级，写出时这些子级会被丢掉。编辑器保证不会产生这种结构（见 ARCHITECTURE.md 的不变量）。

## 保存失败与恢复

写入使用同目录的唯一临时文件，文件同步后再重命名；Unix 平台还同步父目录。文档库写锁获取失败时拒绝修改，仍可只读列出笔记。检测到其他实例修改正文时不会覆盖磁盘内容，可以重新载入或另存副本。

系统退出阶段保存失败时，正文另存至系统本地应用数据目录的 `quiet-md/recovery/<文档库标识>/`。下次打开同一文档库时创建恢复副本，成功写入后移除恢复文件。它是退出失败时的恢复措施，不是完整版本历史。

导出本地资源时检查规范化路径，拒绝文档库外的文件和越界符号链接。HTML 内嵌图片读取上限为 20 MiB，Markdown 复制的单个资源上限为 256 MiB；缺失、被拒绝或超限的 Markdown 资源会计入导出警告。行内附件与块附件均导出为链接并复制文件。
