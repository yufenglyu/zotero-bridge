# Zotero Search Bridge (zsb)

Zotero 外部快速检索与定位工具：从 Zotero Local API 增量同步文献元数据到本地
SQLite FTS5 索引，并通过 `.url`（Windows / Listary）、Alfred、Raycast（macOS）
在外部搜索文献、一键定位到 Zotero 条目。

设计文档：[`spec.md`](spec.md)（本项目以该文档为需求与架构基线）。

## 架构

```
Zotero Local API (localhost:23119)
        │
        ▼
  增量同步引擎 (zsb-sync)
        │
        ├───────────────┐
        ▼               ▼
SQLite FTS5        文件镜像管理器 (zsb-mirror)
(zsb-index)            │
        │              ├─ Windows .url  → Listary
        │              └─ macOS .webloc → Spotlight/Finder
        │
        ├─ zsb CLI（search / open / sync / doctor …）
        ├─ Alfred Script Filter（--format alfred）
        └─ Raycast 扩展（--format json）
```

## 构建

```sh
cargo build --release        # 生成 target/release/zsb.exe
cargo test --workspace       # 55 个单元/集成测试
```

## 快速开始

```sh
zsb doctor            # 检查 Zotero、Local API、FTS5、镜像目录
zsb sync              # 首次全量同步（之后自动增量）
zsb sync --watch      # 常驻同步，默认每 15 秒轮询
zsb search "燃气轮机 转子"
zsb search "author:Smith year:2024" --format json
zsb open --library user --key N49R8KAQ
zsb status
```

维护命令：`zsb rebuild` / `optimize` / `verify-index` / `clean-mirrors`。

## 搜索语法

```
燃气轮机 转子            # 多词 AND（FTS5 trigram 子串匹配）
李                      # 1–2 字符自动回退 LIKE 查询
author:Smith turbine   # 字段限定：author / year / tag / type / library
"exact phrase"         # 双引号短语
```

## Windows + Listary

1. `zsb sync`（或 `zsb sync --watch`）会在镜像目录自动维护 `.url` 文件：
   `%LOCALAPPDATA%\ZoteroSearchBridge\mirrors\windows\`
2. 在 Listary 索引设置中收录该目录（或把配置 `mirror.windows.directory`
   改到你已有的索引目录，如 `D:\Data\ZoteroLinks`）。
3. 在 Listary 中按 作者 / 年份 / 标题 / Item Key 搜索，双击 `.url`
   即可定位到 Zotero 条目。

## macOS + Alfred / Raycast

- Alfred：Script Filter 调用 `zsb search "$1" --format alfred --limit 30`，
  回车执行 `open "{query}"`，见 [`integrations/alfred/`](integrations/alfred/)。
- Raycast：扩展调用 `zsb search --format json`，
  见 [`integrations/raycast/`](integrations/raycast/)。
- 可选 `.webloc` 镜像默认关闭（`mirror.macos.enabled = false`）。

## 配置

默认路径（Windows）：`%APPDATA%\ZoteroSearchBridge\config.toml`
（macOS：`~/Library/Application Support/ZoteroSearchBridge/config.toml`）。
首次运行自动生成，字段见 spec 第 18 节。修改采用“临时文件 + 原子替换”。

数据与索引：`%LOCALAPPDATA%\ZoteroSearchBridge\data\index.sqlite`，
与 Zotero 的 `zotero.sqlite` 完全独立，本项目只读 Local API，绝不写 Zotero。

## 里程碑状态

- [x] **M0** 技术验证：Local API 探测、FTS5 trigram 中文子串搜索、`.url`/`.webloc`
      生成、`zotero://select` 定位（已对真实 Zotero 10 验证通过）
- [x] **M1** 核心索引与增量同步：Server ID 实例隔离、`since` 增量、
      `/deleted` 删除同步、回收站处理、CLI 搜索/打开
- [x] **M2** Windows Listary 集成：`.url` 模板/清理/原子写入、
      mirror_jobs 持久化任务队列（崩溃可恢复）、重试
- [x] **M3**（部分）Alfred/Raycast 输出格式与集成示例
- [x] **M4** Tauri 桌面托盘程序：托盘菜单（显示/立即同步/暂停/退出）、
      状态页、设置页、诊断页、后台轮询同步、开机自启、关闭最小化到托盘
- [x] **M5**（核心）Windows NSIS/MSI 安装包（已构建验证）、GitHub Actions
      CI/发布流水线、签名与公证流程文档、版本迁移机制、故障诊断文档；
      macOS DMG 需 macOS 环境构建

## 桌面程序（M4）

```sh
cd apps/desktop
npm install
npm run build            # 产出 dist/（Tauri 加载它）
cd ../..
cargo build --release -p zsb-desktop
# 运行：target/release/zsb-desktop(.exe)
```

前端联调：`npm run dev` 启动 Vite 开发服务器（浏览器预览会提示无
Tauri 环境，属正常；完整功能需运行桌面程序）。

## 兼容性说明：Zotero 10.0-beta 的对象版本

部分 Zotero 10.0 beta 构建的 Local API 不返回对象版本
（`format=versions` 的值为空串、`Last-Modified-Version: 0`）。
针对这类实例，同步引擎自动回退为“全键列表 + content-hash 比对”策略：
每轮列出全部条目 key、批量拉取、仅写入内容真正变化的条目，
并通过对比远端/本地 key 集合检测删除。功能行为与版本增量同步一致，
代价是每轮同步的读取量更大。稳定版 Zotero 会返回真实版本号，
此时自动使用 spec 第 12 节的标准增量同步。

## 隐私

所有数据保存在本地；不启动公网服务、不上传文献数据、不收集遥测、
不需要 Zotero Web API Key、不直接读写 `zotero.sqlite`（spec 第 20 节）。
`localhost:23119` 不应被转发到局域网或公网。

## License

MIT OR Apache-2.0
