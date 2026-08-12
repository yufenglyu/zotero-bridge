# Zotero Bridge

Zotero Bridge 是一个 Zotero 本地增强工具，用来同步本地文献索引、生成可搜索链接文件，并备份/迁移 Zotero 与插件设置。你可以用 Listary、Finder、Spotlight 或文件管理器快速搜索链接文件，也可以把 `prefs.js` 中的设置项结构化迁移到另一台电脑。

项目不会上传文献数据，不需要 Zotero Web API Key，也不会直接读写 `zotero.sqlite`。所有同步都通过 Zotero Local API 完成。

## 功能

- “索引链接”页：同步 Zotero 文献、维护本地 SQLite FTS5 索引、生成链接文件，并在“运行概览”中集中查看索引、链接、Zotero 实例和文献库状态。
- “设置备份”页：从指定 Zotero profile 的 `prefs.js` 扫描设置，按 Zotero 本身、插件、Zotero 框架和未知来源展开查看、筛选、编辑、备份和导入。
- “设置”页：只保留基础运行配置，包括 Zotero Local API、轮询周期、日志级别、个人库/群组库范围和配置目录入口。
- 自动生成和刷新 Windows `.url`、macOS `.webloc` 链接文件，用于 Listary、Finder、Spotlight、Alfred、Raycast 或文件管理器检索。
- 默认跟随 Zotero 附件命名模板，也支持自定义链接文件命名模板和 URI 模板。
- 支持链接文件状态检查，显示本地文件数、应有文件数、最近更新时间，以及缺失、孤立或过期状态。
- 支持备份、导入、预览确认、编辑、删除和写回 `prefs.js` 设置项，并在导入时逐项迁移路径。
- 诊断 Zotero 连接、索引、链接目录和日志状态，诊断结果以弹窗显示。

## 使用前准备

1. 启动 Zotero。
2. 在 Zotero 中启用本地通信：

   ```text
   设置 -> 高级 -> 允许此计算机上的其他应用程序与 Zotero 通信
   ```

3. 启动 `zotero-bridge`。
4. 在“索引链接”页点击“立即同步”。

Zotero 视图中的项目数可能大于 Zotero Bridge 的“已索引条目”。这是预期行为：Zotero Bridge 只索引顶层文献条目，顶层附件、笔记和批注不会进入索引。

## Windows

默认链接文件目录：

```text
%LOCALAPPDATA%\ZoteroBridge\mirrors\windows\
```

推荐用法：

1. 在 Zotero Bridge 中完成首次同步。
2. 打开“索引链接”页的“链接文件目录”，确认链接文件已经生成。
3. 在 Listary 中把该目录加入索引。
4. 之后可直接在 Listary 中搜索作者、年份、标题或 Item Key，打开结果即可定位到 Zotero。

链接文件目录、命名模板和 URI 模板都在“索引链接”页的“链接文件”区域修改，输入后会自动保存。链接文件数量和健康状态在“运行概览”中查看，待写入和失败任务显示在底部状态栏。修改目录、命名模板或 URI 模板后，点击“刷新链接”可按当前索引重新补写、覆盖、改名和清理链接文件。

## macOS

默认链接文件目录：

```text
~/Zotero Links
```

推荐用法：

1. 在 Zotero Bridge 中完成首次同步。
2. 打开“索引链接”页的“链接文件目录”，确认链接文件已经生成。
3. 使用 Finder 或 Spotlight 搜索该目录中的链接文件。
4. 打开 `.webloc` 文件即可定位到 Zotero。

如果你使用 Alfred、Raycast 或其他启动器，可以把 `~/Zotero Links` 加入对应工具的文件搜索范围。链接文件目录可以在“索引链接”页的“链接文件”区域修改。

## 同步与链接刷新

“立即同步”会访问 Zotero Local API，读取文献变化，更新本地索引，并处理本轮变化涉及的链接文件。

“刷新链接”不会重新读取 Zotero。它会基于当前本地索引、命名模板和 URI 模板重算链接文件，因此适合在修改链接目录、命名模板、URI 模板，或发现本地链接文件缺失、过期、孤立时使用。

“运行概览”保留四类核心信息：索引条目数和最近同步时间、链接文件数量和健康状态、当前 Zotero 实例、文献库数量和启用状态。待写入和失败任务显示在底部状态栏中。

## 命名模板

默认开启“跟随 Zotero 命名模板”。此时 Zotero Bridge 会尽量使用 Zotero 当前附件命名模板生成链接文件名。

关闭“跟随 Zotero 命名模板”后可以使用自定义模板，例如：

```text
{primary_creator} - {year} - {title} -- {item_key}
```

也支持部分 Zotero 模板语法，例如：

```text
{{if itemType == "journalArticle"}}{{authors max="1"}} - {{year}} - {{title}}{{endif}}
```

常用字段包括 `{primary_creator}`、`{year}`、`{title}`、`{item_key}`。
文件名中的非法字符会被替换，避免生成无效路径。

## URI 模板

链接文件默认写入 `{select_uri}`，用于打开 Zotero 并选中对应条目。

可用占位符：

- `{select_uri}`：Zotero 选择条目的 URI。
- `{item_key}` / `{itemKey}`：Zotero Item Key。
- `{title}`：文献标题。

普通用户保持默认即可。

## 设置备份与导入

“设置备份”页用于迁移 Zotero 与插件保存在 `prefs.js` 中的偏好项。它不会整文件覆盖 `prefs.js`，而是把设置结构化备份为 JSON，并在导入时按 key 合并写回。

如果本机有多个 Zotero profile，可以在“本机配置”中浏览或填写对应的 `prefs.js` 文件路径，也可以填写 profile 目录。留空时使用 Zotero Bridge 自动识别到的默认 profile。“本机配置”既是备份源，也是导入目标。

备份时会：

- 导出 Zotero 原生设置和插件设置。
- 按 Zotero 本身、插件、Zotero 框架和未知来源在“设置详览”中显示设置项数量。
- 展开任一分组后显示具体设置项明细，可在当前分组内按类型和关键字筛选，并可一键清空搜索词。
- 支持编辑或从本次草稿删除设置项；点击“备份”会导出草稿，点击“写回”会把草稿保存回源 `prefs.js`。
- 自动识别本地路径值，包括 Zotero 和插件里的路径配置；JSON、locale、Zotero 样式 URL、文件扩展名列表等非路径值不会进入路径迁移。
- 标记疑似敏感项，例如密码、token、cookie、账号和认证相关设置；这些项会进入备份和恢复草稿，请按个人资料处理备份文件。

导入时会：

- 点击“导入”后先生成预览弹窗，显示新增、修改、不变和路径跳过数量；确认后才会写入本机 `prefs.js`。
- 默认导入路径设置项；如果不希望导入备份里的路径，可在导入确认弹窗中取消“导入路径设置项”。
- 在“路径迁移”中逐项查看备份里的旧路径，并为当前机器填写新路径；填写后的新路径会覆盖对应旧路径后导入。
- 旧版本备份文件中如果存在误分类的路径项，加载时会重新分类，避免非路径项出现在路径迁移列表中。
- 写回前自动备份当前 `prefs.js` 为 `.bak` 文件。

导入或写回前请先关闭 Zotero。Zotero 运行时可能在退出时重写 `prefs.js`，因此 Zotero Bridge 会阻止在 Zotero 正在运行时修改本地 `prefs.js`。

## 默认路径

Windows：

```text
配置：%APPDATA%\ZoteroBridge\config.toml
数据：%LOCALAPPDATA%\ZoteroBridge\data\index.sqlite
日志：%LOCALAPPDATA%\ZoteroBridge\logs\zotero-bridge.log
链接：%LOCALAPPDATA%\ZoteroBridge\mirrors\windows\
```

macOS：

```text
配置：~/Library/Application Support/ZoteroBridge/config.toml
数据：~/Library/Application Support/ZoteroBridge/data/index.sqlite
日志：~/Library/Logs/ZoteroBridge/zotero-bridge.log
链接：~/Zotero Links
```

可通过环境变量覆盖配置和数据库路径：

```text
ZOTERO_BRIDGE_CONFIG
ZOTERO_BRIDGE_DATABASE
```

## 开发

仓库结构：

```text
apps/
  desktop/              桌面程序，Vue + Tauri
crates/
  core/                 配置、路径、模型、错误类型
                        prefs.js 备份、分类、导入预览与写回
  zotero-api/           Zotero Local API 客户端
  index/                SQLite/FTS5 本地索引
  sync/                 增量同步、标准化、链接刷新
  mirror/               链接文件生成与任务执行
  launcher/             zotero:// URI 启动
migrations/             SQLite schema 迁移
packaging/              发布所需静态模板
scripts/                本地构建与发布脚本
.github/workflows/      CI 与 GitHub Release 流水线
assets/prefs.js         Zotero prefs.js 参考样本，用于解析回归测试
```

前端构建：

```sh
cd apps/desktop
npm install
npm run build
```

Rust 构建与测试：

```sh
cargo build --release -p zotero-bridge
cargo test --workspace
```

Windows 本地打包：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release-windows.ps1
```

macOS 本地打包：

```sh
bash scripts/release-macos.sh
```

推送 `v*` tag 会触发 GitHub Actions 构建 Windows 和 macOS 桌面产物，并发布到 GitHub Release。

## 隐私

Zotero Bridge 只在本机保存配置、索引、日志、链接文件和用户主动导出的设置备份。不启动公网服务，不收集遥测，不上传文献库内容。设置备份会包含并标记疑似敏感项，导出的 JSON 可能包含你的插件偏好、本机路径和账号相关设置，请按个人资料处理。请不要把 `localhost:23119` 转发到局域网或公网。

## License

MIT OR Apache-2.0
