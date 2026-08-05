# HANDOFF — Zotero Search Bridge 项目交接

更新时间：2026-08-04
仓库：`D:\Data\Downloads\zotget`（git，3 次提交，最新 `9330f42` 之后另有便携版提交）
需求与架构基线：`spec.md`（M0–M5 全部已落地）

## 当前任务

按 `spec.md` 实现 Zotero 外部快速检索与定位工具（Rust + SQLite FTS5 +
Zotero Local API + Tauri），并已完成 M0–M5 全部里程碑、Windows 安装包
和免安装便携版。

## 已完成内容

### M0–M3：核心（提交 `42ea177`）

- **Rust workspace**：6 个 crate（`zsb-core` / `zsb-zotero-api` / `zsb-index`
  / `zsb-sync` / `zsb-mirror` / `zsb-launcher`）+ CLI（`apps/cli`，二进制 `zsb`）。
- **zsb CLI**：`search`（plain/json/alfred 三种格式）、`open` / `open-uri`、
  `sync`（`--full` / `--library` / `--watch`）、`status`、`doctor`、`rebuild`、
  `optimize`、`verify-index`、`clean-mirrors`。
- **索引**：SQLite FTS5 trigram 外部内容表 + 三类触发器；BM25 权重
  10/6/4/3/2.5/2/0.5/0.5；字段限定（author/year/tag/type/library）；
  1–2 字符 LIKE 回退；全参数绑定。
- **同步**：`Zotero-Server-ID` 实例隔离（含 legacy 回退）、`since` 增量、
  `/deleted` 删除、回收站处理、content-hash 去重、版本不稳定退避重试、
  单事务提交 + mirror_jobs 持久化 outbox。
- **镜像**：`.url` / `.webloc` 模板与清理、原子写入、崩溃可恢复重试。
- **测试**：57 项单元 + mock 集成测试全绿，clippy 零警告。
- **真机验证**（Zotero 10.0-beta.22）：索引 4,818 条文献、生成 4,818 个
  `.url`、`doctor` 8 项全 OK、`open` 成功定位条目、二次同步 +0 条。

### M4：Tauri 桌面程序（提交 `b089478`）

- `apps/desktop`（Vue3 + TS + Vite 前端，Tauri 2 后端）。
- 托盘菜单（显示/立即同步/暂停/退出）、状态页、设置页、诊断页、
  后台 15 秒轮询同步、开机自启、关闭最小化到托盘、文件日志。
- 真机截图验证渲染与实时数据正常。

### M5：安装包与发布（提交 `9330f42`）

- Windows 安装包已构建：`target\release\bundle\nsis\…_x64-setup.exe`
  （5.9 MB）与 `…_x64_en-US.msi`（7.7 MB），WebView2 随包分发。
- GitHub Actions：`.github/workflows/ci.yml`（双平台 fmt/clippy/test +
  前端构建）、`release.yml`（tag 触发 CLI + 桌面安装包 + 草稿发布，
  Apple 公证 secrets 已接线）。
- 文档：`docs/release.md`（签名/公证/版本迁移/检查单）、
  `docs/troubleshooting.md`（故障诊断表）。

### 便携版 + 打包脚本（2026-08-05）

- 便携版 zip 改为输出到 `target\dist\zsb-portable-v0.1.0-windows-x64.zip`，
  不再提交进 git；新增 `scripts\release.ps1`（构建前端 + release 二进制 +
  组装 zip + 复制 NSIS/MSI 安装包，`-SkipBuild` 可跳过构建）。
- 静态打包资源在 `packaging\`（说明.txt、zsb-config.toml，git 跟踪的源文件）；
  脚本从 `target\release\` 取 exe 组装到 `target\portable\` 再压缩。
- 坑：Windows PowerShell 5.1 把无 BOM 的 UTF-8 脚本按 GBK 解析，
  中文注释会导致莫名其妙的语法错误 → 脚本统一用纯 ASCII 编写。
- **2026-08-05 改名**：资源目录原叫 `dist-portable\`，名字像构建产物被
  误删过两次 → `git mv` 改为 `packaging\`；release.ps1 在目录缺失时给出
  明确报错（提示 `git checkout -- packaging` 恢复）。

### 桌面端修复与 UI 重构（2026-08-05）
- **常驻命令行窗口**：main.rs 缺 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`，
  release exe 是 console 子系统 → 已修复，新 exe PE subsystem = 2 (GUI)。
- **主题**：CSS 变量双主题（VS Code Light+/Dark+ 近似配色），
  `data-theme` 挂在 `<html>`；浅色/深色/跟随系统三档，localStorage
  `zsb-theme-mode` 持久化；跟随系统 = matchMedia change 监听 + 每秒轮询兜底。
- **分组重构**：状态页 = 概览卡片 / 同步控制（按钮收进 panel）/
  文献库 / 目录与日志（可点击行）；设置页 = 外观 / 常规 / Zotero 连接 /
  搜索与索引 / 镜像（Win+macOS 合并）/ 存储，新增吸底保存栏与脏标记
  （设置 tab 上的橙点）；诊断页加说明文字。
- 设置页补齐了此前未暴露的字段：log_level、api_base、request_timeout、
  maximum_limit、index_extra、short_query_fallback。

### /deleted 404 导致同步全挂（2026-08-05 傍晚修复）

- 现象：状态页文献库报错 `zotero API error: status 404 No endpoint found`，
  版本停在 0。
- 根因：同一台 Zotero 10.0-beta.22 现在 `format=versions` 返回真实版本
  （Last-Modified-Version: 30，早前为空 → 走 FullScan 回退不碰 /deleted），
  引擎改走 Stable 路径后调用 `/users/0/deleted`，而该 beta 的 Local API
  **不暴露 /deleted**（404），错误经 `?` 传播使整个同步轮失败。
- 修复：`LocalApiClient::deleted_objects` 对 404 特判——视为端点不支持，
  记 `tracing::warn` 并返回空集合（last_modified_version=0，引擎比较时
  已会忽略 0）；删除检测由 FullScan 的 key 差集路径兜底。
- 测试：client.rs 新增 2 项（404 容忍 / 500 仍报错，内嵌 TcpListener
  stub server），workspace 64 项全绿；真实 Zotero 端到端 `zsb sync` 通过
  （+4797 条，版本 0→30，仅一条 /deleted WARN）。
- **重要**：用户实际运行的便携版在 `target\dist\zsb-portable\`（自行解压，
  data\ 在里面），修复后的 exe 已原地替换该目录下的两个 exe（2026-08-05
  17:21），用户重启程序即可，其 zsb-config.toml（有自定义）未动。
  **该目录后来被用户删除**（19:1x），之后不再原地替换，只更新 zip。

### Zotero 文件名模板 + 快捷方式刷新（2026-08-05 晚）

- **`zsb_mirror::ztemplate`**：Zotero 7 附件重命名同款模板语法渲染器
  （`{{if/elseif/else/endif}}`（可嵌套）、`field == "v"` / `!=` / 裸字段真值、
  `{{authors max="1" initialize="given"}}`、`replaceFrom/replaceTo/regexOpts="g"`）。
  模板含 `{{` 即按 Zotero 语法解析（`filename::render_auto`），否则走旧
  `{placeholder}` 语法。取数自条目 `raw_json` 的完整 `data`（**ItemData 加了
  `#[serde(flatten)] other` 保留全部字段**——此前 edition/number/versionNumber
  会被 serde 丢弃，注释谎称"preserved"实际没有）；raw_json 缺失时回退到
  IndexedItem 规范化列。
- Zotero 模板**不强制 item_key 后缀**；重名时引擎自动追加 ` -- <key>`
  （`filename::with_key_suffix`），plan_mirror_jobs 与 refresh 共用同一
  消歧逻辑。长度超限硬截断到 180 字符。
- **refresh-mirrors**：`zsb_sync::refresh_mirrors`（CLI `zsb refresh-mirrors`、
  桌面端「刷新快捷方式」按钮）——全量重渲染文件名，改名变化的（Rename）、
  磁盘缺失的补写（Create），存储名与任务同事务提交；新增
  `db.items_for_library` 全量行查询。
- **改名**：界面「镜像/镜像目录」→「快捷方式/快捷方式目录」（配置文件键名
  不变）。设置页模板输入框改多行 textarea。
- 测试：ztemplate 10 项 + 引擎碰撞/refresh 2 项，workspace 76 项全绿。
- 坑：css 链式选择器编辑易截断规则链（.field input:focus 曾被误截），改
  样式后必须回读检查。
- 注意：**存量 raw_json 缺 flatten 前的字段**（edition/number/versionNumber），
  需全量重同步（或 rebuild）后 Zotero 模板才能取到这些字段；此间模板渲染
  优雅回退为空串。

### 模板跟随 Zotero + 4 字改名（2026-08-05 深夜）

- **`zsb_core::zotero_prefs`**：从 Zotero profile 读命名模板。定位
  `%APPDATA%\Zotero\Zotero\profiles.ini` → Default profile → prefs.js →
  `user_pref("extensions.zotero.attachmentRenameTemplate", ...)`（JS 转义
  \" \\ \n 还原）。macOS 路径 `~/Library/Application Support/Zotero/`。
- **resolve_template 优先级**：自定义模板（非空且 ≠ 旧默认串）>
  Zotero pref > 内置默认。**配置留空或仍是旧默认串 = 跟随 Zotero**
  （新装默认即跟随；老配置里存的旧默认串自动视为跟随）。
- 设置页改为「跟随 Zotero 命名模板」复选（默认勾选，展示 Zotero 模板
  只读预览，新 Tauri 命令 `zotero_template`）；取消勾选才出现自定义
  textarea。
- 界面改名：快捷方式目录→**快捷目录**、刷新快捷方式→**刷新链接**、
  待写入快捷方式→**待写入链接**、面板→**链接定位（Listary / Alfred）**。
- 坑：引擎测试原来用内置默认模板，跟随功能后测试会读到开发机真实
  Zotero pref → 环境依赖。解法：测试固定使用「渲染结果相同但字符串不同」
  的自定义模板（`{title}{container_title}` 空占位符技巧）。

### 自定义路径（配置与索引，2026-08-04 晚）

- `zsb_core::paths` 新增解析 API：`resolve_config_file` /
  `resolve_database_file` / `is_portable_config` / `portable_config_file`，
  优先级：CLI flag > 环境变量（`ZSB_CONFIG` / `ZSB_DATABASE`）>
  配置 `[storage].database`（经 `expand_path`，支持 `%VAR%`/`~`）>
  便携默认（exe 旁 `data\index.sqlite`）> 平台默认。
- **便携模式**：exe 旁存在 `zsb-config.toml` 即触发；配置即用该文件，
  索引默认落 exe 旁 `data\`。免安装 zip 自带一个注释版 `zsb-config.toml`，
  实现真便携（已实测：解压后 `status` 显示数据库在解压目录 `data\`，
  `ZSB_DATABASE` 与 `--database` 覆盖均生效）。
- 配置文件新增 `[storage] database` 字段（空 = 默认）；
  桌面端设置页新增“存储”面板可编辑，提示改后需重启。
- 接入点：CLI `apps/cli/src/main.rs` 与桌面端 `apps/desktop/src-tauri/src/main.rs`
  的 `main()` 开头；日志目录不受便携模式影响（仍写 `%LOCALAPPDATA%`）。
- 测试：paths.rs 新增 5 项解析优先级测试，workspace 共 62 项全绿。

## 卡住的问题 / 未解决事项

1. **Zotero 10.0-beta 无对象版本**：`format=versions` 返回空串、
   `Last-Modified-Version: 0`。已用“全量 key 列表 + content-hash 比对 +
   key 差集检测删除”回退（自动检测，无需配置）。代价：每次同步都全量
   拉取（4,819 条约 60 秒），`zsb sync --watch` 的 15 秒轮询在该 beta 上
   偏重。待 Zotero 稳定版验证标准增量路径。
2. **macOS 全部未实测**：DMG 构建、签名公证、`.webloc`、Alfred/Raycast
   只有配置与文档，需要 macOS 机器。
3. **签名未配置**：Windows signtool/Azure Trusted Signing、Apple 证书
   均属密钥资产，流程与 CI 接线已备好但未真正签过。
4. **自动更新通知**（spec M4/M5 尾部）未实现，需要更新服务器。
5. **M3 的 Raycast 扩展**只有 README 与示例代码，未生成可安装的扩展包。

## 下一步计划（建议顺序）

1. 用户提供 GitHub 远端 → 推送仓库与 `v0.1.0` 标签，触发 release
   流水线出草稿发布（把便携版 zip 一并附加）。
2. 便携版 zip 纳入 release.yml 自动生成（当前为手工打包）。
3. Zotero 稳定版上验证标准增量同步（`since` 路径），确认回退自动解除。
4. 干净 Windows 机器做安装包验收：安装 → 首次同步 → Listary 搜索 →
   双击定位（spec §23 验收标准逐条过）。
5. macOS 环境构建 DMG、实测 Alfred/Raycast、`.webloc`。
6. 可选增强（spec §25）：拼音搜索、PDF 全文、集合路径、
   `zotero://open-pdf` 附件直开。

## 踩过的坑（重要）

1. **Zotero 10.0-beta.22 的 Local API 版本字段为空**（`"version": ""`、
   `Last-Modified-Version: 0`）——DTO 必须容忍数字/空串双格式
   （`de_u64_lenient`），否则整个同步链解析失败。
2. **`directories` crate 的 ProjectDirs 在 Windows 上会产生
   `Roaming\ZoteroSearchBridge\ZoteroSearchBridge\data` 双层目录**，
   且 config/data 都落在 Roaming，不符合 spec §7（config→%APPDATA%、
   data→%LOCALAPPDATA%）。已改为手工拼路径（`crates/core/src/paths.rs`）。
3. **rusqlite `Connection` 是 `Send` 但 `!Sync`**：Tauri 命令/后台任务
   要求 `Send` future，持有 `&Database` 的同步引擎不满足。解法：同步
   跑在独立 blocking 线程 + current-thread runtime（`sync_once_send`）。
4. **Windows 上 `explorer.exe <uri>` 成功也返回退出码 1**：`open` crate
   会误判失败。改用 `rundll32 url.dll,FileProtocolHandler` 启动
   `zotero://` 链接。
5. **FTS 查询参数绑定顺序必须与 SQL 占位符顺序一致**（先全部 LIKE 再
   全部等值条件），不能按查询词出现顺序绑定——曾导致
   `year:2021 转子` 这类混合查询无结果，已加回归测试。
6. **LIKE 子句每个列一个占位符**，参数要按列数重复绑定（曾出现
   “Wrong number of parameters”）。
7. **Tauri 2 debug 构建会加载 devUrl 而非 frontendDist**：不跑 Vite
   开发服务器时窗口显示“localhost 拒绝连接”。解法：tauri.conf.json
   里不配 devUrl（debug/release 都用 dist）。
8. **`cargo test` 不会刷新 `target/debug/zsb.exe`**：改完代码验证 CLI
   必须先 `cargo build`，否则测的是旧二进制。
9. **Windows 保留文件名与路径规则**：`CON/PRN/COM1…`、结尾空格/句点、
   重命名需“先建后删”、`fs::rename` 不能覆盖已存在文件（已在 mirror
   层处理，勿回归）。
10. **打包时 WebView2 引导器下载偶发 TLS 中断**：`npx tauri build`
    重试即可（已遇到一次，第二次成功）。
11. **Zotero 模板 `authors` 的 `name` 参数默认是 `family`（只取姓）**：
    `{{authors max="1" initialize="given"}}` 在两段式创作者上只输出姓，
    `initialize="given"` 因 given 部分未被包含而无效；单字段（合并）
    创作者整体视为姓原样输出。曾错误实现为「名取首字母+姓」
    （`W. Wang`），已按官方文档
    （zotero.org/support/file_renaming）重写 `format_creators`，
    补全 `name`/`initialize`/`initialize-with`/`name-part-separator`/
    `join` 参数（2026-08-05）。
12. **新增字段保留后旧索引行不会自动修复**：`ItemData` 的 flatten map
    （保留 number 等全部字段）是后加的，旧行 raw_json 缺字段，而
    content_hash 不含 raw_json、增量同步按版本号抓——旧行永远不会被
    重写（standard 链接文件名曾因此缺标准号前缀）。解法：
    `NORMALIZER_VERSION` 元数据机制（不一致则强制一次全量同步）+
    跳过条件补 `raw_json` 比对（engine.rs，2026-08-05）。以后凡是
    normalizer/DTO 保留字段变化，递增该版本号。
13. **Zotero 模板的字段名走「基字段映射」**：`{{publisher}}` 对 thesis
    实际取 `university`、`{{date}}` 对 patent 实际取 `issueDate`（不是
    filingDate）。resolve() 已加候选链：publisher→[publisher,
    university, institution]、date→[date, issueDate, filingDate]
    （ztemplate.rs，2026-08-05）。验证方法：以 Zotero 已重命名的附件
    文件名为基准逐字比对（注意 children 列表无 filename，要取完整条目
    的 path 字段），60 条抽查 52 条一致，8 条不一致均为 Zotero 侧
    旧模板/旧元数据重命名的遗留文件。

## 关键路径速查

- 数据目录：`%LOCALAPPDATA%\ZoteroSearchBridge\`（`data\` `logs\` `mirrors\windows\`）
- 配置：`%APPDATA%\ZoteroSearchBridge\config.toml`
- 当前实例 ID：`YdjFPFScJZZp`（Zotero 10.0-beta.22，库约 4,819 条）
- release 二进制：`target\release\zsb.exe`、`target\release\zsb-desktop.exe`
- 安装包：`target\release\bundle\{nsis,msi}\`
- 构建工具链：Rust 在 `C:\Users\LYF\.cargo\bin`（Git Bash 需手动加 PATH），
  npm 在 `C:\Program Files\nodejs`（Git Bash 需手动加 PATH）。
