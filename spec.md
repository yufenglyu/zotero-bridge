# Zotero 外部快速检索与定位工具

## 项目规划与详细实现文档

**项目代号：** Zotero Search Bridge
**建议仓库名：** `zotero-search-bridge`
**文档版本：** 1.0
**编制日期：** 2026年8月4日
**目标平台：** Windows 11、macOS
**核心技术：** Rust、SQLite FTS5、Zotero Local API、Tauri、Alfred/Raycast

# 1. 项目背景

当前的 Zotero 外部搜索流程是：

1. Zotero 插件读取文献条目；
2. 按“作者－标题”生成 `.url` 文件；
3. `.url` 内容指向：

```
zotero://select/library/items/N49R8KAQ
```

1. Listary 对 `.url` 文件目录建立索引；
2. 用户在 Listary 中搜索文件名并双击；
3. Windows 调用 Zotero，定位到对应条目。

该方案可以获得很好的搜索体验，但存在以下问题：

- Zotero 新增文献后需要重新导出；
- 修改标题或作者后需要重新生成文件；
- 删除文献后旧 `.url` 文件仍可能残留；
- 每次完整导出都会重复处理整个文献库；
- Windows 和 macOS 需要分别维护不同实现；
- 当前插件导出逻辑、文件搜索逻辑和 Zotero 数据结构耦合较深。

本项目将把这一流程改造成一个独立、自动、跨平台的本地服务。

# 2. 项目目标

## 2.1 核心目标

系统应实现：

1. 从 Zotero Local API 读取个人文献库和群组文献库；
2. 首次运行时建立完整的本地文献索引；
3. 自动检测新增、修改、删除和移入回收站的条目；
4. 仅同步发生变化的条目，不重复导出整个文献库；
5. 在 Zotero 未运行时仍可搜索已经建立的本地索引；
6. 通过 `zotero://select/...` 链接启动 Zotero 并定位条目；
7. Windows 保留 Listary 搜索、双击打开的使用方式；
8. macOS 支持 Alfred、Raycast，并可选生成 `.webloc` 文件；
9. 所有数据和索引均保存在本地；
10. 不直接修改或依赖 Zotero 的 `zotero.sqlite` 表结构。

Zotero Local API 运行在 `localhost:23119/api/`，直接从本地文献库提供数据，不经过互联网。读取本地库时不需要 Zotero Web API Key，但 Zotero 必须正在运行并允许其他本地应用通信。项目只使用读取请求，即使较新的 Zotero Local API 已开始支持授权后的写入操作，也不在本项目范围内。 citeturn510753search0turn910425view1

## 2.2 性能目标

以包含约 5 万个顶层文献条目的文献库为基准：

| 指标                  | 目标             |
| --------------------- | ---------------- |
| 普通搜索响应时间 P95  | 不超过 100 ms    |
| 默认同步轮询周期      | 15 秒            |
| Zotero 修改到索引可见 | 通常不超过 30 秒 |
| 搜索结果默认数量      | 30 条            |
| 单次条目批量读取      | 最多 50 条       |
| 后台空闲内存目标      | 不超过 100 MB    |
| 无 Zotero 运行时搜索  | 可用             |
| 网络依赖              | 无               |

这些是本项目的验收目标，不是 Zotero 本身的性能保证。

## 2.3 非目标

第一版不实现：

- 修改、创建或删除 Zotero 条目；
- 替代 Zotero 自身的同步功能；
- 全文 OCR；
- PDF 内容全文索引；
- 云端文献库；
- 手机端应用；
- 引文格式化或 Word 引用插入；
- Listary 内部插件或动态结果提供器；
- Zotero 数据库修复。

# 3. 技术方案比较

## 3.1 方案一：直接读取 `zotero.sqlite`

优点：

- Zotero 未启动时仍可读取；
- 不需要 HTTP 请求；
- 可以访问数据库中的底层字段。

缺点：

- Zotero 数据库表结构属于内部实现；
- 不同 Zotero 版本可能修改表结构；
- 标题、作者、标签、条目类型分散在多张表中；
- 群组库 ID 与用于链接的 group ID 需要额外转换；
- 并发访问、文件锁和数据库副本管理较复杂；
- 直接写入可能造成数据库损坏。

Zotero 官方允许以只读方式访问 `zotero.sqlite`，但明确建议优先使用 Web API、Local API 或内部 JavaScript API，并指出数据库结构可能随版本改变，绝不能直接修改数据库。 citeturn510753search3

**结论：** 不作为正式方案，只保留为未来的诊断或数据恢复工具。

## 3.2 方案二：每次搜索直接调用 Zotero Local API

优点：

- 实现简单；
- 不需要维护本地数据库；
- 搜索结果始终来自当前 Zotero 文献库。

缺点：

- Zotero 必须处于运行状态；
- 每次键入都请求 Local API；
- 无法直接被 Listary 当作文件结果展示；
- 无法在 Zotero 关闭时搜索；
- 不适合作为 Alfred、Raycast、Listary 的统一数据源。

**结论：** 适合作为原型，不适合作为最终架构。

## 3.3 方案三：Local API + 本地索引 + 平台适配器

流程如下：

```
Zotero Local API
        │
        ▼
增量同步服务
        │
        ├───────────────┐
        ▼               ▼
SQLite FTS5         文件镜像管理器
本地搜索索引         │
        │             ├─ Windows .url
        │             └─ macOS .webloc
        │
        ├─ CLI
        ├─ Alfred
        ├─ Raycast
        └─ 后续原生搜索窗口
```

优点：

- 不依赖 Zotero 内部数据库结构；
- 支持增量同步；
- Zotero 关闭后仍可搜索；
- Windows 可继续使用 Listary；
- macOS 可提供真正的动态搜索结果；
- 同一套核心逻辑支持多个平台；
- 后续可以增加 PDF 全文、拼音搜索等功能。

**结论：采用方案三。**

# 4. 总体架构

系统由六个核心部分组成。

## 4.1 Zotero API 客户端

职责：

- 检测 Zotero 是否运行；
- 检测 Local API 是否开启；
- 获取 API 版本；
- 获取 Zotero 实例 ID；
- 发现个人库和群组库；
- 获取条目版本；
- 批量获取条目数据；
- 获取删除记录。

启动时请求：

```
GET http://localhost:23119/api/
```

读取响应头：

```
Zotero-API-Version
Zotero-Schema-Version
Zotero-Server-ID
```

Zotero 10 及更高版本的 Local API 会通过 `Zotero-Server-ID` 标识当前 Zotero 数据库实例。不同实例的对象版本不可混用，因此本项目必须按照 Server ID 隔离缓存；如果请求返回 `412 Precondition Failed`，应停止当前同步并为新实例建立独立索引分区。 citeturn910425view1turn910425view3

## 4.2 增量同步引擎

职责：

- 执行首次完整同步；
- 按库保存最后同步版本；
- 获取发生变化的条目；
- 获取删除的条目；
- 处理同步过程中出现的并发修改；
- 把文件系统操作写入持久化任务队列。

## 4.3 本地数据与搜索索引

使用一个独立 SQLite 数据库保存：

- Zotero 实例信息；
- 文献库信息；
- 标准化文献元数据；
- 同步版本；
- 文件镜像状态；
- FTS5 全文索引；
- 待执行文件操作。

本地索引与 Zotero 的 `zotero.sqlite` 完全独立。

## 4.4 文件镜像管理器

根据本地索引自动生成：

```
Windows：*.url
macOS：*.webloc
```

当文献标题、作者或年份变化时，自动重命名文件；当条目删除或移入回收站时，自动删除镜像文件。

## 4.5 搜索与启动接口

提供统一 CLI：

```
zsb search <关键词>
zsb open <library> <item-key>
zsb sync
zsb status
zsb doctor
zsb rebuild
```

其中 `zsb` 是 `Zotero Search Bridge` 的命令行程序名。

## 4.6 桌面管理界面

使用 Tauri 构建轻量托盘程序，提供：

- 同步状态；
- 立即同步；
- 暂停同步；
- 重建索引；
- 打开镜像目录；
- 修改文件名模板；
- 启用或禁用群组库；
- 设置开机启动；
- 查看日志；
- 检查 Zotero Local API。

Tauri 提供 Windows 和 macOS 的系统托盘与自动启动能力，macOS 自动启动可以使用 LaunchAgent。 citeturn306347search0turn306347search4

# 5. 技术栈

## 5.1 核心语言

使用 Rust。

选择原因：

- 可编译为单个本地程序；
- Windows 和 macOS 共用大部分代码；
- 适合常驻后台；
- 内存占用可控；
- 文件系统和 SQLite 操作能力较强；
- CLI、后台服务和 Tauri 后端可复用同一个核心库。

## 5.2 建议依赖

```
tokio
reqwest
serde
serde_json
rusqlite
thiserror
tracing
tracing-subscriber
directories
clap
sha2
regex
unicode-normalization
url
open
uuid
time
```

桌面端另外使用：

```
tauri
tauri-plugin-autostart
```

SQLite 建议使用 `rusqlite` 的 bundled 模式，把兼容 FTS5 的 SQLite 一并打包，避免依赖用户系统中的 SQLite 版本。

## 5.3 前端

Tauri 设置窗口可以使用：

```
Vue 3
TypeScript
Vite
```

前端只负责设置和状态显示，所有同步、索引和文件操作均放在 Rust 核心层。

# 6. 项目目录结构

```
zotero-search-bridge/
├─ Cargo.toml
├─ README.md
├─ LICENSE
├─ crates/
│  ├─ core/
│  │  ├─ src/config.rs
│  │  ├─ src/models.rs
│  │  ├─ src/errors.rs
│  │  └─ src/lib.rs
│  ├─ zotero-api/
│  │  ├─ src/client.rs
│  │  ├─ src/dto.rs
│  │  ├─ src/discovery.rs
│  │  └─ src/lib.rs
│  ├─ index/
│  │  ├─ src/database.rs
│  │  ├─ src/migrations.rs
│  │  ├─ src/search.rs
│  │  └─ src/lib.rs
│  ├─ sync/
│  │  ├─ src/engine.rs
│  │  ├─ src/normalizer.rs
│  │  ├─ src/state.rs
│  │  └─ src/lib.rs
│  ├─ mirror/
│  │  ├─ src/filename.rs
│  │  ├─ src/windows_url.rs
│  │  ├─ src/macos_webloc.rs
│  │  ├─ src/worker.rs
│  │  └─ src/lib.rs
│  └─ launcher/
│     ├─ src/lib.rs
│     └─ src/platform.rs
├─ apps/
│  ├─ cli/
│  │  └─ src/main.rs
│  └─ desktop/
│     ├─ src-tauri/
│     └─ src/
├─ integrations/
│  ├─ alfred/
│  ├─ raycast/
│  └─ listary/
├─ migrations/
│  ├─ 0001_initial.sql
│  └─ 0002_fts.sql
├─ tests/
│  ├─ fixtures/
│  ├─ integration/
│  └─ performance/
└─ docs/
   ├─ architecture.md
   ├─ zotero-api.md
   ├─ search.md
   └─ release.md
```

模块之间不得直接访问彼此内部结构，而应通过明确接口交互。

# 7. 本地目录规划

## 7.1 Windows

```
配置：
%APPDATA%\ZoteroSearchBridge\config.toml

数据库：
%LOCALAPPDATA%\ZoteroSearchBridge\data\index.sqlite

日志：
%LOCALAPPDATA%\ZoteroSearchBridge\logs\

Listary 镜像：
%LOCALAPPDATA%\ZoteroSearchBridge\mirrors\windows\
```

也允许用户把镜像目录设置到：

```
D:\Data\ZoteroLinks\
```

## 7.2 macOS

```
配置和数据库：
~/Library/Application Support/ZoteroSearchBridge/

日志：
~/Library/Logs/ZoteroSearchBridge/

可选 .webloc 镜像：
~/Zotero Links/
```

macOS 默认推荐 Alfred 或 Raycast 直接搜索 SQLite，不默认生成数万个 `.webloc` 文件。

# 8. 数据库设计

## 8.1 Zotero 实例表

```
CREATE TABLE zotero_instances (
    server_id       TEXT PRIMARY KEY,
    api_base        TEXT NOT NULL,
    api_version     INTEGER,
    schema_version  INTEGER,
    first_seen_at   TEXT NOT NULL,
    last_seen_at    TEXT NOT NULL,
    is_active       INTEGER NOT NULL DEFAULT 1
);
```

`server_id` 取自 `Zotero-Server-ID`。

对于不提供该响应头的旧版 Zotero：

```
legacy:<本机安装UUID>
```

同时在设置界面提供“当前 Zotero 数据库已更换，重新建立索引”功能。

## 8.2 文献库表

```
CREATE TABLE libraries (
    id                  INTEGER PRIMARY KEY,
    server_id           TEXT NOT NULL,
    library_kind        TEXT NOT NULL,
    zotero_library_id   TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    api_prefix          TEXT NOT NULL,
    last_version        INTEGER NOT NULL DEFAULT 0,
    enabled             INTEGER NOT NULL DEFAULT 1,
    last_sync_at        TEXT,
    last_error          TEXT,

    FOREIGN KEY(server_id)
        REFERENCES zotero_instances(server_id),

    UNIQUE(server_id, library_kind, zotero_library_id)
);
```

字段示例：

个人库：

```
library_kind      = user
zotero_library_id = 0
api_prefix        = /users/0
```

群组库：

```
library_kind      = group
zotero_library_id = 123456
api_prefix        = /groups/123456
```

## 8.3 文献条目表

```
CREATE TABLE items (
    id                  INTEGER PRIMARY KEY,
    library_id          INTEGER NOT NULL,
    item_key            TEXT NOT NULL,
    item_version        INTEGER NOT NULL,
    item_type           TEXT NOT NULL,

    title               TEXT NOT NULL DEFAULT '',
    creators            TEXT NOT NULL DEFAULT '',
    primary_creator     TEXT NOT NULL DEFAULT '',
    year                TEXT NOT NULL DEFAULT '',
    container_title     TEXT NOT NULL DEFAULT '',
    tags                TEXT NOT NULL DEFAULT '',
    abstract_note       TEXT NOT NULL DEFAULT '',
    extra               TEXT NOT NULL DEFAULT '',

    date_modified       TEXT,
    select_uri          TEXT NOT NULL,
    mirror_filename     TEXT,
    content_hash        TEXT NOT NULL,
    raw_json            TEXT,

    indexed_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,

    FOREIGN KEY(library_id)
        REFERENCES libraries(id)
        ON DELETE CASCADE,

    UNIQUE(library_id, item_key)
);
```

建议保留 `raw_json`，用于：

- 调试字段映射；
- 后续索引新字段；
- 数据库迁移；
- 避免为了增加一个搜索字段而立即重新请求全部条目。

可以提供配置项关闭 `raw_json`，以减少数据库体积。

## 8.4 文件任务表

```
CREATE TABLE mirror_jobs (
    id              INTEGER PRIMARY KEY,
    operation       TEXT NOT NULL,
    platform        TEXT NOT NULL,
    old_path        TEXT,
    new_path        TEXT,
    content         TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    retry_count     INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
```

`operation` 可取：

```
create
replace
rename
delete
```

这张表实现持久化 Outbox 模式：数据库更新和文件任务在同一个事务中提交，文件系统操作随后异步执行。即使程序在写文件前退出，重启后也能继续执行未完成任务。

# 9. FTS5 搜索索引

## 9.1 建表

```
CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    primary_creator,
    creators,
    year,
    container_title,
    tags,
    abstract_note,
    extra,
    content='items',
    content_rowid='id',
    tokenize='trigram case_sensitive 0'
);
```

使用外部内容表可以避免 FTS5 再保存一份完整元数据，但必须通过触发器保持 `items` 和 `items_fts` 一致。SQLite 官方文档也建议使用触发器维护外部内容表。 citeturn975559search0

## 9.2 触发器

```
CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN
    INSERT INTO items_fts(
        rowid,
        title,
        primary_creator,
        creators,
        year,
        container_title,
        tags,
        abstract_note,
        extra
    )
    VALUES (
        new.id,
        new.title,
        new.primary_creator,
        new.creators,
        new.year,
        new.container_title,
        new.tags,
        new.abstract_note,
        new.extra
    );
END;
```

更新和删除触发器按 FTS5 外部内容表的标准方式实现。

增加维护命令：

```
INSERT INTO items_fts(items_fts) VALUES('rebuild');
INSERT INTO items_fts(items_fts, rank) VALUES('integrity-check', 1);
INSERT INTO items_fts(items_fts) VALUES('optimize');
```

## 9.3 中文搜索策略

默认使用 FTS5 `trigram` tokenizer，而不是 `unicode61`。

`trigram` 会为连续三个字符建立索引，适合：

- 中文标题的任意子串；
- 英文单词中间部分；
- 作者姓名；
- 中英文混合标题；
- DOI、型号和标识符片段。

SQLite 官方说明，trigram tokenizer 支持一般性的子串匹配，但少于三个 Unicode 字符的全文查询不会匹配结果。 citeturn611014view1turn611014view3

因此搜索策略分为两类：

```
查询长度 ≥ 3 个字符：
使用 FTS5 MATCH

查询长度为 1～2 个字符：
使用 LIKE 回退查询
```

例如搜索：

```
AI
李
热
```

使用：

```
SELECT *
FROM items
WHERE title LIKE '%' || ? || '%'
   OR creators LIKE '%' || ? || '%'
   OR tags LIKE '%' || ? || '%'
LIMIT 30;
```

## 9.4 排序权重

搜索结果使用 FTS5 的 BM25 排序，建议权重：

| 字段         | 权重 |
| ------------ | ---- |
| 标题         | 10.0 |
| 第一作者     | 6.0  |
| 全部作者     | 4.0  |
| 年份         | 3.0  |
| 期刊或出版物 | 2.5  |
| 标签         | 2.0  |
| 摘要         | 0.5  |
| Extra        | 0.5  |

查询示例：

```
SELECT
    items.*,
    items_fts.rank
FROM items_fts
JOIN items ON items.id = items_fts.rowid
WHERE items_fts MATCH ?
  AND rank MATCH 'bm25(10.0, 6.0, 4.0, 3.0, 2.5, 2.0, 0.5, 0.5)'
ORDER BY rank
LIMIT ?;
```

SQLite FTS5 的 BM25 数值越小表示匹配度越高，并允许为不同列设置不同权重。 citeturn611014view0

## 9.5 查询语法

MVP 支持普通关键词：

```
燃气轮机 转子
Smith turbine
数字孪生 2024
```

增强版本支持字段限定：

```
author:Smith turbine
year:2024 数字孪生
tag:仿真
type:journalArticle
library:MyLibrary
```

查询解析器需要：

1. 识别双引号短语；
2. 对 FTS5 特殊字符转义；
3. 防止用户输入直接成为 SQL；
4. 使用参数绑定；
5. 把多个普通词转换为 AND 查询；
6. 对少于三个字符的词使用后置 LIKE 过滤。

# 10. Zotero 条目标准化

## 10.1 条目过滤

从：

```
GET /api/users/0/items/top
```

获取顶层条目。

默认排除：

```
attachment
note
annotation
```

保留普通文献类型，如：

```
journalArticle
book
bookSection
conferencePaper
thesis
report
webpage
patent
document
computerProgram
```

独立笔记和独立附件是否加入索引，可以作为高级选项。

## 10.2 标题

```
data.title
```

如果为空：

```
[无标题] -- ITEMKEY
```

## 10.3 作者

把 `creators` 规范化为：

```
张三; 李四; Wang, Wei
```

处理两种格式：

```
{
  "firstName": "Wei",
  "lastName": "Wang"
}
```

以及机构作者：

```
{
  "name": "World Health Organization"
}
```

第一作者选择顺序：

1. `creatorType = author`；
2. 没有作者时使用 editor；
3. 没有 editor 时使用第一个 creator；
4. 全部为空时使用“无作者”。

## 10.4 年份

从 `data.date` 中提取第一个合理的四位年份：

```
\b(1[5-9]\d{2}|20\d{2}|21\d{2})\b
```

无法提取时保留为空，不把完整日期直接作为文件名年份。

## 10.5 出版物字段

按优先级合并：

```
publicationTitle
bookTitle
proceedingsTitle
encyclopediaTitle
dictionaryTitle
university
institution
publisher
```

最终统一保存为：

```
container_title
```

## 10.6 标签和摘要

标签：

```
data.tags[].tag
```

摘要：

```
data.abstractNote
```

所有字符串执行：

- Unicode NFKC 规范化；
- 删除控制字符；
- 合并连续空白；
- 去除首尾空格；
- 保留原始大小写用于显示；
- 搜索时执行不区分大小写匹配。

# 11. Zotero 定位链接生成

个人库：

```
zotero://select/library/items/N49R8KAQ
```

群组库：

```
zotero://select/groups/123456/items/N49R8KAQ
```

Zotero 团队确认个人库应使用 `zotero://select/library/items/<itemKey>`，群组库使用 `zotero://select/groups/<groupID>/items/<itemKey>`。 citeturn693308search0turn693308search2turn693308search6

实现函数：

```
fn build_select_uri(
    library_kind: LibraryKind,
    library_id: &str,
    item_key: &str,
) -> String
```

禁止根据本地 SQLite 的 `libraryID` 直接拼接群组链接。

# 12. 增量同步设计

## 12.1 首次同步

每个库从版本 `0` 开始：

```
GET <prefix>/items/top?since=0&format=versions&includeTrashed=1
```

响应示意：

```
{
  "N49R8KAQ": 315,
  "ABCDEF12": 318
}
```

然后每 50 个 key 为一批：

```
GET <prefix>/items?itemKey=N49R8KAQ,ABCDEF12&includeTrashed=1
```

条目响应中已经包含：

- creators；
- tags；
- collections；
- relations；
- item version。

Zotero 官方同步流程推荐先使用 `format=versions` 获取变化对象，再按 key 分批读取完整对象，批量请求最多可按 50 个 key 组织。 citeturn936980view3

## 12.2 后续同步

设本地库版本为：

```
last_version = 318
```

获取变化条目：

```
GET <prefix>/items/top?since=318&format=versions&includeTrashed=1
```

获取删除条目：

```
GET <prefix>/deleted?since=318
```

删除响应包含：

```
{
  "collections": [],
  "searches": [],
  "items": [
    "N49R8KAQ"
  ],
  "tags": []
}
```

`since` 参数只返回指定库版本之后发生变化的对象，`/deleted?since=` 返回被删除对象的 key。 citeturn936980view1turn936980view2

## 12.3 同步状态机

```
Idle
  │
  ▼
Probe Zotero
  │
  ├─ Zotero 未运行 ──► Offline
  │
  ├─ API 未开启 ────► ConfigurationError
  │
  ▼
Discover Instance
  │
  ▼
Discover Libraries
  │
  ▼
Fetch Changed Versions
  │
  ▼
Fetch Changed Items
  │
  ▼
Fetch Deleted Keys
  │
  ▼
Check Stable Library Version
  │
  ├─ 版本发生变化 ──► Retry
  │
  ▼
Commit Database Transaction
  │
  ▼
Process Mirror Jobs
  │
  ▼
Idle
```

## 12.4 并发修改处理

同步开始时记录：

```
start_version
```

分别检查变化条目请求和删除请求返回的：

```
Last-Modified-Version
```

如果多个响应返回的库版本不同，说明用户在同步过程中仍在修改 Zotero 数据。

处理方式：

1. 不提交新的 `last_version`；
2. 已获得的数据可以丢弃或保留在内存；
3. 等待短暂退避；
4. 从原 `last_version` 重新同步；
5. 连续失败后进入延迟重试。

Zotero 官方同步文档要求比较各响应的 `Last-Modified-Version`，如果同步期间版本改变，应重新获取变化和删除数据。 citeturn936980view3

## 12.5 数据库事务

一次稳定同步在单个事务中完成：

```
BEGIN

1. 插入或更新变化条目
2. 删除已删除或已移入回收站条目
3. 写入 mirror_jobs
4. 更新 libraries.last_version
5. 更新 libraries.last_sync_at

COMMIT
```

文件操作不在数据库事务中直接执行，避免文件写入失败导致 SQLite 长时间占用写锁。

# 13. 文件镜像设计

## 13.1 文件名模板

默认：

```
{primary_creator} - {year} - {title} -- {item_key}
```

Windows 示例：

```
张三 - 2024 - 燃气轮机转子动力学研究 -- N49R8KAQ.url
```

macOS 示例：

```
张三 - 2024 - 燃气轮机转子动力学研究 -- N49R8KAQ.webloc
```

用户可配置：

```
{title} - {primary_creator}
{year} - {primary_creator} - {title}
{primary_creator} - {title}
```

文件名必须始终保留：

```
-- {item_key}
```

以避免重名和便于清理旧文件。

## 13.2 文件名清理

跨平台统一替换：

```
< > : " / \ | ? *
```

同时处理：

- 控制字符；
- 行换行；
- Windows 末尾空格；
- Windows 末尾句点；
- `CON`、`PRN`、`AUX`、`NUL`；
- `COM1` 到 `COM9`；
- `LPT1` 到 `LPT9`。

建议最终基础文件名不超过约 180 个字符，并优先保留：

```
作者 + 年份 + 标题开头 + item key
```

## 13.3 Windows `.url`

内容：

```
[InternetShortcut]
URL=zotero://select/library/items/N49R8KAQ
```

写入流程：

1. 生成新内容；
2. 写入同目录临时文件；
3. 刷新文件；
4. 原子重命名到目标文件；
5. 成功后删除旧文件；
6. 更新 mirror job 为 completed。

当元数据改变时：

```
先创建新文件
再删除旧文件
```

这样可以避免程序中断后链接完全消失。

## 13.4 macOS `.webloc`

内容：

```
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>URL</key>
  <string>zotero://select/library/items/N49R8KAQ</string>
</dict>
</plist>
```

macOS 可以通过系统 URL 打开机制启动相应 URL Scheme 的应用。 citeturn554301search6turn554301search7

# 14. Windows 与 Listary 集成

## 14.1 默认集成方式

Listary 继续搜索普通 `.url` 文件。

设置步骤：

1. 在程序中启用“Windows URL 镜像”；
2. 选择镜像目录；
3. 在 Listary 索引设置中确保该目录被收录；
4. 搜索标题、作者或年份；
5. 双击 `.url` 文件；
6. Zotero 打开并定位条目。

Listary 的公开功能以文件搜索、应用启动和命令行为主，因此第一版采用文件镜像，而不依赖未公开的动态结果提供接口。这个选择是基于其公开能力作出的工程判断。 citeturn554301search2

## 14.2 Listary 搜索体验

可以使用：

```
燃气轮机
张三 2024
Smith turbine
N49R8KAQ
```

由于文件名包含：

```
作者 + 年份 + 标题 + Item Key
```

无需 Listary 理解 Zotero 数据结构。

## 14.3 后续增强

后续可以增加 Listary 自定义命令：

```
z <关键词>
```

该命令启动项目自身的搜索窗口，而不是生成文件搜索结果。但这属于增强功能，不作为 MVP 前置条件。

# 15. macOS 集成

## 15.1 首选：Alfred Script Filter

Alfred Script Filter 可以由脚本动态返回搜索结果，并推荐使用 JSON 格式。每个结果可以提供 title、subtitle、arg 和唯一 uid。 citeturn554301search0turn554301search5

调用：

```
zsb search "$1" --format alfred --limit 30
```

输出：

```
{
  "items": [
    {
      "uid": "library:N49R8KAQ",
      "title": "燃气轮机转子动力学研究",
      "subtitle": "张三; 李四 · 2024 · Journal of Turbomachinery",
      "arg": "zotero://select/library/items/N49R8KAQ",
      "valid": true
    }
  ]
}
```

用户回车后执行：

```
open "$1"
```

## 15.2 Raycast 扩展

Raycast 扩展使用 TypeScript：

1. 在输入变化时调用 `zsb search --format json`；
2. 把结果映射为 Raycast `List.Item`；
3. 主操作打开 `select_uri`；
4. 附加操作复制标题、复制 Zotero 链接或在 Finder 中打开附件。

Raycast 支持扩展命令和 deeplink，但本项目只需把 Zotero URI 交给系统打开，无需为 Zotero URI 建立额外协议。 citeturn554301search25turn554301search34

## 15.3 Spotlight 兼容

可选生成 `.webloc` 文件到：

```
~/Zotero Links/
```

优点：

- Finder 和 Spotlight 可以按文件名搜索；
- 不依赖 Alfred 或 Raycast。

缺点：

- 文献库很大时会产生大量文件；
- 文件目录可能显得杂乱；
- 搜索排序和元数据展示弱于 Alfred/Raycast。

因此 `.webloc` 默认关闭。

# 16. CLI 设计

## 16.1 搜索

```
zsb search "燃气轮机 转子"
```

输出：

```
1. 燃气轮机转子动力学研究
   张三; 李四 · 2024
   zotero://select/library/items/N49R8KAQ
```

JSON：

```
zsb search "燃气轮机" --format json
```

Alfred：

```
zsb search "燃气轮机" --format alfred
```

## 16.2 打开条目

```
zsb open-uri "zotero://select/library/items/N49R8KAQ"
```

或者：

```
zsb open --library user --key N49R8KAQ
```

## 16.3 同步

```
zsb sync
zsb sync --full
zsb sync --library user
zsb sync --library group:123456
```

## 16.4 诊断

```
zsb doctor
```

输出检查：

```
[OK] Zotero process reachable
[OK] Local API enabled
[OK] API version: 3
[OK] Server ID detected
[OK] SQLite FTS5 available
[OK] Mirror directory writable
[OK] zotero:// URI handler registered
```

## 16.5 维护

```
zsb rebuild
zsb optimize
zsb verify-index
zsb clean-mirrors
zsb status
```

# 17. Rust 接口设计

## 17.1 Zotero 数据源接口

```
#[async_trait]
pub trait ZoteroSource {
    async fn probe(&self) -> Result<ServerInfo>;

    async fn list_libraries(
        &self,
    ) -> Result<Vec<RemoteLibrary>>;

    async fn changed_item_versions(
        &self,
        library: &RemoteLibrary,
        since: u64,
    ) -> Result<VersionResponse>;

    async fn fetch_items(
        &self,
        library: &RemoteLibrary,
        keys: &[String],
    ) -> Result<ItemResponse>;

    async fn deleted_objects(
        &self,
        library: &RemoteLibrary,
        since: u64,
    ) -> Result<DeletedResponse>;
}
```

## 17.2 索引存储接口

```
pub trait ItemRepository {
    fn library_state(
        &self,
        library: LibraryId,
    ) -> Result<LibraryState>;

    fn apply_sync_batch(
        &mut self,
        batch: SyncBatch,
    ) -> Result<()>;

    fn search(
        &self,
        query: SearchQuery,
    ) -> Result<Vec<SearchResult>>;

    fn rebuild_fts(&mut self) -> Result<()>;
}
```

## 17.3 文件镜像接口

```
pub trait MirrorBackend {
    fn platform(&self) -> Platform;

    fn extension(&self) -> &'static str;

    fn build_content(
        &self,
        item: &IndexedItem,
    ) -> Result<Vec<u8>>;

    fn process_job(
        &self,
        job: &MirrorJob,
    ) -> Result<()>;
}
```

## 17.4 启动接口

```
pub trait UriLauncher {
    fn open(&self, uri: &str) -> Result<()>;
}
```

这些接口保证：

- API 模块可以使用模拟服务器测试；
- 搜索索引可以独立测试；
- Windows 和 macOS 文件生成互不影响；
- 后续可增加 Linux 适配，而不修改同步核心。

# 18. 配置文件

```
[app]
poll_interval_seconds = 15
start_at_login = true
log_level = "info"

[zotero]
api_base = "http://localhost:23119/api"
request_timeout_seconds = 10
include_user_library = true
group_mode = "all"

[search]
default_limit = 30
maximum_limit = 100
index_abstract = true
index_extra = true
store_raw_json = true
short_query_fallback = true

[mirror.windows]
enabled = true
directory = "%LOCALAPPDATA%/ZoteroSearchBridge/mirrors/windows"
template = "{primary_creator} - {year} - {title} -- {item_key}"

[mirror.macos]
enabled = false
directory = "~/Zotero Links"
template = "{primary_creator} - {year} - {title} -- {item_key}"

[maintenance]
optimize_after_updates = 5000
retain_logs_days = 14
```

设置修改应通过“写临时文件后替换”的方式保存，避免配置文件部分写入。

# 19. 错误处理

## 19.1 Zotero 未运行

错误：

```
Connection refused
```

处理：

- 不清空本地索引；
- 搜索继续可用；
- 同步状态显示“等待 Zotero 启动”；
- 按退避周期重试；
- 不频繁弹出通知。

## 19.2 Local API 未启用

Local API 被关闭时会返回 `403 Forbidden`。 citeturn510753search0turn910425view2

界面提示：

```
请在 Zotero 中打开：
设置 → 高级 →
允许此计算机上的其他应用程序与 Zotero 通信
```

## 19.3 Zotero 数据库切换

检测到新的：

```
Zotero-Server-ID
```

处理：

1. 暂停旧实例同步；
2. 建立新实例分区；
3. 执行完整同步；
4. 文件镜像切换到新实例；
5. 旧实例索引暂时保留；
6. 提示用户选择删除或保留旧实例数据。

禁止把旧实例 `last_version` 用于新实例。

## 19.4 文件占用

文件创建、重命名或删除失败时：

- mirror job 保持 pending；
- `retry_count + 1`；
- 保存错误信息；
- 采用指数退避；
- 不回滚已经完成的 Zotero 元数据同步；
- 超过重试上限后在托盘中显示警告。

## 19.5 FTS 索引不一致

处理命令：

```
zsb verify-index
zsb rebuild
```

重建步骤：

1. 暂停同步；
2. 对 `items_fts` 执行 rebuild；
3. 执行 integrity-check；
4. 恢复同步。

## 19.6 损坏的本地数据库

启动时执行：

```
PRAGMA quick_check;
```

失败时：

1. 关闭损坏数据库；
2. 把原文件重命名为带时间戳的备份；
3. 创建新数据库；
4. 重新从 Local API 建立索引；
5. 不操作 Zotero 原始数据库。

# 20. 日志与隐私

## 20.1 日志原则

日志默认只记录：

- Server ID 前几位；
- 文献库 ID；
- 条目数量；
- item key；
- 同步版本；
- 请求状态；
- 文件任务状态；
- 错误类型。

默认不记录：

- 完整摘要；
- 完整笔记；
- PDF 内容；
- API 返回的完整 JSON；
- 用户搜索历史。

## 20.2 本地安全

Local API 只应通过 loopback 使用，不应把 `localhost:23119` 转发到局域网或公网。Zotero 官方也明确提醒 Local API 无需读取认证，因此不得暴露该端口。 citeturn510753search0

本项目：

- 不启动公网服务；
- 不上传文献数据；
- 不收集遥测；
- 不保存 Zotero 账号密码；
- 不要求 Zotero Web API Key；
- 不直接读写 `zotero.sqlite`。

# 21. 测试方案

## 21.1 单元测试

覆盖：

- 作者格式化；
- 年份提取；
- Unicode 规范化；
- 文件名非法字符；
- Windows 保留文件名；
- 文件名长度截断；
- 个人库 URI；
- 群组库 URI；
- 条目类型过滤；
- 查询转义；
- 短查询回退；
- 配置读取；
- content hash。

## 21.2 API 集成测试

使用本地模拟 HTTP 服务器覆盖：

- 正常初始同步；
- 增量新增；
- 标题修改；
- 作者修改；
- 移入回收站；
- 永久删除；
- 群组库新增；
- `403 Forbidden`；
- `412 Precondition Failed`；
- 请求超时；
- 返回无效 JSON；
- 同步期间库版本改变；
- 50 条批量边界；
- 空文献库。

## 21.3 数据库测试

覆盖：

- 数据库迁移；
- FTS 触发器；
- 插入后可搜索；
- 更新后旧标题不可搜索；
- 删除后结果消失；
- rebuild；
- integrity-check；
- mirror job 崩溃恢复；
- 切换 Server ID；
- 多库同 item key。

## 21.4 平台测试

Windows：

- `.url` 双击；
- Zotero 未启动时打开；
- Zotero 已启动时定位；
- Listary 按标题搜索；
- Listary 按作者搜索；
- 文件名修改后旧链接删除；
- 开机启动。

macOS：

- Alfred Script Filter；
- Raycast 搜索；
- `open zotero://...`；
- `.webloc` 双击；
- LaunchAgent 自动启动；
- Apple Silicon；
- Intel Mac 可选支持。

## 21.5 性能测试

生成测试数据集：

```
1,000 条
10,000 条
50,000 条
100,000 条
```

记录：

- 初始建库时间；
- FTS 数据库大小；
- 单条更新耗时；
- 1、2、3、10 字符查询耗时；
- 中文、英文、混合查询耗时；
- 进程空闲内存；
- 30 条结果查询 P50、P95、P99。

# 22. 项目阶段规划

## M0：技术验证

交付内容：

- Local API 探测程序；
- 获取个人库前 10 条文献；
- 生成一条 `.url`；
- 生成一条 `.webloc`；
- 验证 Windows 和 macOS 能打开 Zotero；
- 验证 SQLite FTS5 trigram 可用；
- 验证中文标题子串搜索。

退出条件：

```
能够从 Local API 获得条目并从外部定位到 Zotero。
```

## M1：核心索引和增量同步

交付内容：

- Rust Workspace；
- 配置模块；
- Zotero API 客户端；
- Server ID 分区；
- libraries/items 表；
- FTS5 索引；
- 首次完整同步；
- `since` 增量同步；
- `/deleted` 删除同步；
- CLI 搜索；
- CLI 打开条目。

退出条件：

```
新增、修改、删除条目均能自动反映到本地搜索结果。
```

## M2：Windows Listary 集成

交付内容：

- Windows `.url` 生成；
- 文件名模板；
- 文件名清理；
- mirror_jobs；
- 文件任务重试；
- Listary 配置说明；
- 从旧插件导出方式迁移。

退出条件：

```
Listary 可以搜索自动维护的链接文件，并双击定位 Zotero 条目。
```

## M3：macOS 集成

交付内容：

- Alfred Workflow；
- Raycast 扩展；
- 可选 `.webloc`；
- macOS URI 启动；
- macOS 开机启动。

退出条件：

```
macOS 可以通过 Alfred 或 Raycast 按标题和作者检索并打开 Zotero。
```

## M4：Tauri 桌面管理程序

交付内容：

- 托盘图标；
- 状态页；
- 设置页；
- 立即同步；
- 重建索引；
- 打开目录；
- 查看日志；
- 自动启动；
- 更新通知。

退出条件：

```
普通用户无需命令行即可配置和维护系统。
```

## M5：发布和质量保障

交付内容：

- Windows 安装程序；
- macOS 安装包；
- macOS 签名和公证流程；
- Windows 签名流程；
- GitHub Actions；
- 自动测试；
- 版本迁移；
- 发布文档；
- 故障诊断文档。

退出条件：

```
能够在全新 Windows 和 macOS 环境中完成安装、首次同步和搜索。
```

# 23. 验收标准

项目第一版完成时必须满足：

1. 不直接访问或修改 `zotero.sqlite`；
2. 可以发现 Zotero Local API；
3. 可以索引个人库；
4. 可以选择性索引群组库；
5. 新增条目可自动加入索引；
6. 修改标题或作者后搜索结果更新；
7. 删除或移入回收站后搜索结果消失；
8. Windows `.url` 自动创建、重命名和删除；
9. Listary 可以通过文件名搜索文献；
10. 双击结果可以定位到 Zotero；
11. macOS Alfred 或 Raycast 可以动态搜索；
12. Zotero 关闭时本地搜索仍可用；
13. 切换 Zotero 数据库不会污染原有索引；
14. 程序崩溃后未完成文件任务可以恢复；
15. 本地索引可以重建；
16. 所有数据库查询使用参数绑定；
17. 默认不上传任何文献数据；
18. 50,000 条数据的普通搜索 P95 达到目标；
19. Windows 和 macOS 均有可安装版本；
20. 具备升级时的数据库迁移机制。

# 24. 从现有插件迁移

建议不要直接覆盖旧导出目录。

迁移流程：

1. 保留现有 Zotero 插件；
2. 安装 Zotero Search Bridge；
3. 选择一个新的镜像目录；
4. 执行首次同步；
5. 把新目录加入 Listary 索引；
6. 随机验证至少 20 条文献；
7. 在 Zotero 中分别测试新增、修改和删除；
8. 确认新目录自动更新；
9. 从 Listary 索引中移除旧目录；
10. 删除旧 `.url` 导出目录；
11. 禁用或卸载原 Zotero 导出插件。

不建议混用新旧目录，否则无法判断旧文件是有效记录还是历史残留。

# 25. 后续扩展

在核心同步稳定后，可增加：

## 25.1 拼音搜索

增加：

```
title_pinyin
creators_pinyin
```

例如：

```
szls
shuziluansheng
```

匹配：

```
数字孪生
```

## 25.2 PDF 全文搜索

读取 Zotero 已建立的附件全文索引，把 PDF 正文加入单独的 FTS 表。

该功能应与文献元数据索引分开，避免摘要和 PDF 正文降低标题搜索的排序质量。较新的 Local API 也提供本地全文相关端点，但必须继续按照 Server ID 隔离版本。 citeturn910425view1

## 25.3 集合路径

保存条目所属集合：

```
数字孪生 / 仿真 / 燃气轮机
```

支持：

```
collection:燃气轮机
```

## 25.4 Better BibTeX Citation Key

从 Extra 字段或 Better BibTeX 提供的接口读取 citation key，支持：

```
smith2024turbine
```

搜索和打开。

该功能不应成为核心依赖，以保证未安装 Better BibTeX 时程序仍可使用。

## 25.5 独立快速搜索窗口

增加类似 Listary、Raycast 的原生全局搜索：

```
Alt + Z
```

功能：

- 即时搜索；
- 键盘选择；
- 回车定位；
- `Ctrl/Cmd + Enter` 打开附件；
- 复制 Zotero 链接；
- 复制标题；
- 复制 citation key。

## 25.6 附件打开

对附件条目生成：

```
zotero://open-pdf/library/items/<attachmentKey>
```

支持直接打开 PDF，而不是只选择父级文献。

# 26. 实施优先级总结

第一阶段只实现完整闭环：

```
Local API
→ 增量同步
→ SQLite FTS5
→ Windows .url
→ Listary
→ Zotero 定位
```

第二阶段增加：

```
CLI
→ Alfred
→ Raycast
```

第三阶段再增加：

```
Tauri 托盘
→ 设置界面
→ 安装程序
→ 自动升级
```

不应在第一阶段提前实现：

- PDF 全文；
- 拼音分词；
- 自定义搜索窗口；
- Better BibTeX 深度集成；
- Zotero 写入；
- 云同步。

最终核心原则是：

```
Zotero 是数据源；
本地 SQLite 是搜索缓存；
.url/.webloc 是平台兼容层；
zotero://select 是定位协议；
所有变化通过版本号增量同步。
```

该文档可以直接作为项目的需求规格和架构设计基线；实际开发应从 **M0 技术验证** 和 **M1 核心索引** 开始，暂不引入完整桌面界面。