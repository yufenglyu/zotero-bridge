# Zotero Bridge

Zotero Bridge 是一个本地桌面工具，用来把 Zotero 文献库同步到本地 SQLite 全文索引，并为每条可检索文献生成本地链接文件。你可以用 Listary、Finder、Spotlight 或文件管理器快速搜索这些链接文件，打开后直接定位到 Zotero 条目。

项目不会上传文献数据，不需要 Zotero Web API Key，也不会直接读写 `zotero.sqlite`。所有同步都通过 Zotero Local API 完成。

## 功能

- 桌面托盘程序，后台轮询 Zotero Local API。
- 增量同步文献元数据到本地 SQLite FTS5 索引。
- 自动生成和刷新 Windows `.url`、macOS `.webloc` 链接文件。
- 默认跟随 Zotero 附件命名模板，也支持自定义链接文件命名模板。
- 支持自定义链接文件 URI 模板。
- 诊断 Zotero 连接、索引、链接目录和日志状态。

## 使用前准备

1. 启动 Zotero。
2. 在 Zotero 中启用本地通信：

   ```text
   设置 -> 高级 -> 允许此计算机上的其他应用程序与 Zotero 通信
   ```

3. 启动 `zotero-bridge`。
4. 在状态页点击“立即同步”。

Zotero 视图中的项目数可能大于 Zotero Bridge 的“已索引条目”。这是预期行为：Zotero Bridge 只索引顶层文献条目，顶层附件、笔记和批注不会进入索引。

## Windows

默认链接文件目录：

```text
%LOCALAPPDATA%\ZoteroBridge\mirrors\windows\
```

推荐用法：

1. 在 Zotero Bridge 中完成首次同步。
2. 打开状态页的“链接文件目录”，确认 `.url` 文件已经生成。
3. 在 Listary 中把该目录加入索引。
4. 之后可直接在 Listary 中搜索作者、年份、标题或 Item Key，打开结果即可定位到 Zotero。

链接文件目录可以在设置页修改。修改目录、命名模板或 URI 模板后，点击状态页“刷新链接”可按当前索引重新补写、覆盖、改名和清理链接文件。

## macOS

默认链接文件目录：

```text
~/Zotero Links
```

推荐用法：

1. 在 Zotero Bridge 中完成首次同步。
2. 打开状态页的“链接文件目录”，确认 `.webloc` 文件已经生成。
3. 使用 Finder 或 Spotlight 搜索该目录中的链接文件。
4. 打开 `.webloc` 文件即可定位到 Zotero。

如果你使用 Alfred、Raycast 或其他启动器，可以把 `~/Zotero Links` 加入对应工具的文件搜索范围。链接文件目录可以在设置页修改。

## 同步与链接刷新

“立即同步”会访问 Zotero Local API，读取文献变化，更新本地索引，并处理本轮变化涉及的链接文件。

“刷新链接”不会重新读取 Zotero。它会基于当前本地索引、命名模板和 URI 模板重算链接文件，因此适合在修改链接目录、命名模板、URI 模板，或发现本地链接文件缺失、过期、孤立时使用。

## 命名模板

默认开启“跟随 Zotero 命名模板”。此时 Zotero Bridge 会尽量使用 Zotero 当前附件命名模板生成链接文件名。

关闭后可以使用自定义模板，例如：

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
  zotero-api/           Zotero Local API 客户端
  index/                SQLite/FTS5 本地索引
  sync/                 增量同步、标准化、链接刷新
  mirror/               链接文件生成与任务执行
  launcher/             zotero:// URI 启动
migrations/             SQLite schema 迁移
packaging/              发布所需静态模板
scripts/                本地构建与发布脚本
.github/workflows/      CI 与 GitHub Release 流水线
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
powershell -ExecutionPolicy Bypass -File scripts\release.ps1
```

macOS 本地打包：

```sh
bash scripts/release-macos.sh
```

推送 `v*` tag 会触发 GitHub Actions 构建 Windows 和 macOS 桌面产物，并发布到 GitHub Release。

## 隐私

Zotero Bridge 只在本机保存配置、索引、日志和链接文件。不启动公网服务，不收集遥测，不上传文献库内容。请不要把 `localhost:23119` 转发到局域网或公网。

## License

MIT OR Apache-2.0
