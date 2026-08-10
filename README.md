# Zotero Bridge

Zotero Bridge 是一个本地文献检索与定位工具。它从 Zotero Local API
增量同步文献元数据到本地 SQLite FTS5 索引，并通过 `.url`
（Windows / Listary）、Alfred、Raycast（macOS）在 Zotero 外部快速搜索文献，
再用 `zotero://select/...` 一键定位到 Zotero 条目。

架构说明见 [docs/architecture.md](docs/architecture.md)。

## 目录结构

```text
apps/
  cli/                  zotero-bridge 命令行程序
  desktop/              Zotero Bridge 桌面托盘程序（Vue + Tauri）
crates/
  core/                 配置、路径、模型、错误类型
  zotero-api/           Zotero Local API 客户端
  index/                SQLite/FTS5 本地索引
  sync/                 增量同步、标准化、链接刷新
  mirror/               .url/.webloc 文件生成与任务执行
  launcher/             zotero:// URI 启动
integrations/
  alfred/ listary/ raycast/  外部启动器集成说明
migrations/             SQLite schema 迁移
packaging/
  portable/             发布输入模板，只放需要进入便携包的静态文件
scripts/
  release.ps1           Windows 本地构建与打包脚本
target/                 Cargo/Tauri 构建、暂存和发布产物；不提交 git
.github/workflows/      CI 与 v* tag 自动发布流水线
```

目录职责约定：

- `apps`、`crates`、`integrations`、`migrations`、`docs`、`scripts` 是源码和文档。
- `packaging/portable` 只保存可维护的便携包模板，例如 `zotero-bridge-config.toml`。
- `target/cargo-release` 是 release 脚本隔离出来的 Cargo 构建目录，避免旧缓存污染。
- `target/portable` 是便携包组装暂存目录。
- `target/dist` 是最终可发布目录，只放 zip、安装包等成品。

## 快速开始

确保 Zotero 正在运行，并在 Zotero 中启用本地通信：

```text
设置 -> 高级 -> 允许此计算机上的其他应用程序与 Zotero 通信
```

CLI 常用命令：

```sh
zotero-bridge doctor
zotero-bridge sync
zotero-bridge sync --watch
zotero-bridge search "燃气轮机 转子"
zotero-bridge search "author:Smith year:2024" --format json
zotero-bridge open --library user --key N49R8KAQ
zotero-bridge status
```

维护命令：

```sh
zotero-bridge rebuild
zotero-bridge optimize
zotero-bridge verify-index
zotero-bridge clean-mirrors
zotero-bridge refresh-mirrors
```

## Windows + Listary

1. 运行 `zotero-bridge sync`，或让桌面程序常驻后台自动同步。
2. 程序会在快捷目录维护 `.url` 文件，默认位置为：
   `%LOCALAPPDATA%\ZoteroSearchBridge\mirrors\windows\`
3. 在 Listary 索引设置中收录该目录。
4. 在 Listary 中搜索作者、年份、标题或 Item Key，双击 `.url` 即可定位到 Zotero。

不建议和旧插件导出的 `.url` 目录混用。迁移时先选择新目录，确认新目录稳定后，
再从 Listary 中移除旧目录。

## 文件名模板

默认跟随 Zotero 自己的附件命名模板。也可以在桌面端设置页关闭“跟随 Zotero
命名模板”，改用自定义模板。

简洁语法示例：

```text
{primary_creator} - {year} - {title} -- {item_key}
```

也支持 Zotero 模板语法，例如：

```text
{{if itemType == "journalArticle"}}{{authors max="1"}} - {{year}} - {{title}}{{endif}}
```

改完模板后运行：

```sh
zotero-bridge refresh-mirrors
```

或在桌面端状态页点击“刷新链接”，让已有链接文件全量改名/补写。
链接文件内的 URI 默认使用 `{select_uri}`，也可在桌面端设置页改为自定义
`uri_template`，常用占位符为 `{select_uri}`、`{item_key}`、`{title}`。

## macOS + Alfred / Raycast

- Alfred：Script Filter 调用
  `zotero-bridge search "$1" --format alfred --limit 30`，回车打开输出的 URL。
- Raycast：扩展调用
  `zotero-bridge search --format json`，并用 `select_uri` 打开 Zotero 条目。
- `.webloc` 镜像默认关闭，可在配置中启用。

## 配置与数据

默认路径：

```text
Windows 配置：%APPDATA%\ZoteroSearchBridge\config.toml
Windows 数据：%LOCALAPPDATA%\ZoteroSearchBridge\data\index.sqlite
Windows 日志：%LOCALAPPDATA%\ZoteroSearchBridge\logs\zotero-bridge.log

macOS 配置/数据：~/Library/Application Support/ZoteroSearchBridge/
macOS 日志：~/Library/Logs/ZoteroSearchBridge/
```

自定义路径优先级：

1. CLI 参数：`--config <路径>` / `--database <路径>`
2. 环境变量：`ZOTERO_BRIDGE_CONFIG` / `ZOTERO_BRIDGE_DATABASE`
3. 配置文件 `[storage].database`
4. 便携模式：exe 旁存在 `zotero-bridge-config.toml`
5. 平台默认路径

仍兼容旧环境变量 `ZSB_CONFIG` / `ZSB_DATABASE` 和旧便携标记
`zsb-config.toml`。

## 便携版

发布 zip 内只包含：

```text
zotero-bridge.exe
zotero-bridge-desktop.exe
zotero-bridge-config.toml
```

使用方式：

1. 解压 zip 到任意目录。
2. 双击 `zotero-bridge-desktop.exe` 运行桌面托盘程序。
3. 首次同步后，把快捷目录加入 Listary 索引。

只要 `zotero-bridge-config.toml` 与 exe 放在同一目录，就启用便携模式：

```text
配置：<解压目录>\zotero-bridge-config.toml
索引：<解压目录>\data\index.sqlite
```

日志仍写入用户目录：

```text
%LOCALAPPDATA%\ZoteroSearchBridge\logs\
```

如需自定义数据库位置，编辑 `zotero-bridge-config.toml`：

```toml
[storage]
database = "D:\\zotero-bridge\\index.sqlite"
```

## 构建

```sh
cargo build --release
cargo test --workspace
```

桌面前端：

```sh
cd apps/desktop
npm install
npm run build
cd ../..
cargo build --release -p zotero-bridge-desktop
```

修改索引、同步或链接命名逻辑后，至少运行 `cargo test --workspace`。

## 打包发布

Windows 本地打包：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release.ps1
```

跳过构建，只重新组装已有 release exe：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -SkipBuild
```

发布成品只从 `target/dist/` 取：

```text
target/dist/zotero-bridge-portable-v<version>-windows-x64.zip
target/dist/*.msi
target/dist/*-setup.exe
```

`scripts/release.ps1` 会：

1. 构建桌面前端。
2. 使用隔离目录 `target/cargo-release` 构建 release exe。
3. 在 `target/portable/zotero-bridge-portable` 组装便携包。
4. 清空并重建 `target/dist`。
5. 只把最终 zip 和安装包复制到 `target/dist`。

推送 `v*` tag 会触发 `.github/workflows/release.yml` 构建 Windows / macOS
产物，并按 `CHANGELOG.md` 中对应版本章节发布正式 GitHub Release。v0.1.0
发布命令示例：

```sh
git tag v0.1.0
git push origin main
git push origin v0.1.0
```

## 已知兼容性

部分 Zotero 10.0 beta 构建的 Local API 不返回对象版本
（`format=versions` 值为空串、`Last-Modified-Version: 0`）。同步引擎会自动回退为
“全键列表 + content-hash 比对”，功能一致但每轮读取量更大。

Zotero 视图数量可能比本地索引多。当前索引按规格排除顶层 `attachment`、`note`
和 `annotation`，所以附件、笔记、批注不会计入“已索引条目”。

## 隐私

所有数据保存在本地；不启动公网服务、不上传文献数据、不收集遥测、
不需要 Zotero Web API Key、不直接读写 `zotero.sqlite`。
不要把 `localhost:23119` 转发到局域网或公网。

## License

MIT OR Apache-2.0
