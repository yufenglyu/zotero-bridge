# 故障诊断指南

先运行 `zotero-bridge doctor`（或桌面程序的“诊断”页），按下表对照处理。

## Zotero 连接类

| 现象 | 原因与处理 |
| --- | --- |
| `Zotero 未运行或不可达` | Zotero 没启动。本地搜索不受影响；同步会在 Zotero 启动后自动恢复（后台循环静默重试）。 |
| `Local API disabled (403)` | Zotero → 设置 → 高级 → 勾选“允许此计算机上的其他应用程序与 Zotero 通信”。 |
| `412 Precondition Failed` | Zotero 数据库实例已更换。程序会为新实例建立独立索引分区并全量重建；旧实例数据保留，可在确认后手动删除索引库重来。 |
| 每次同步都是全量扫描（日志出现 full-scan） | Zotero 10.0-beta 的 Local API 不返回对象版本，属已知兼容回退，功能不受影响；升级 Zotero 稳定版后自动恢复增量同步。 |

## 搜索类

| 现象 | 原因与处理 |
| --- | --- |
| 搜不到刚添加的文献 | 默认 15 秒轮询；运行 `zotero-bridge sync` 或托盘“立即同步”。 |
| 单字/两字搜索无结果但长词有 | 1–2 字符走 LIKE 回退，仅匹配标题/作者/标签；属设计行为。 |
| 结果异常或怀疑索引损坏 | `zotero-bridge verify-index` → 失败则 `zotero-bridge rebuild`。 |
| 搜索完全不可用 | 删除 `%LOCALAPPDATA%\ZoteroSearchBridge\data\index.sqlite` 后 `zotero-bridge sync` 重建（启动时 `PRAGMA quick_check` 失败的损坏库会自动备份为 `*.corrupt-*`）。 |

## 镜像文件类（Listary）

| 现象 | 原因与处理 |
| --- | --- |
| Listary 搜不到 `.url` | 确认镜像目录已加入 Listary 索引：`zotero-bridge status` 查看待执行任务；`zotero-bridge doctor` 检查目录可写。 |
| 托盘显示“失败任务” | 文件被杀软/同步盘占用。关闭占用程序后 `zotero-bridge sync` 会重试 pending 任务。 |
| 残留旧文件 | `zotero-bridge clean-mirrors` 删除索引中不存在的镜像文件。 |
| 改名后新旧文件同时存在 | 重命名是“先建后删”；旧文件删除失败会进入重试队列，重试期间两个文件可能短暂共存。 |

## 桌面程序类

| 现象 | 原因与处理 |
| --- | --- |
| 窗口白屏/无法访问页面 | debug 构建误用 devUrl 的旧版本问题；请重新 `npm run build` 后 `cargo build`。 |
| 关闭窗口后程序“消失” | 设计行为：程序最小化到系统托盘，右键托盘图标可显示窗口或退出。 |
| 开机未自启 | 设置页勾选“开机自动启动”并保存（Windows 写入 Run 注册表项，macOS 安装 LaunchAgent）。 |

## 日志

- 桌面程序：`%LOCALAPPDATA%\ZoteroSearchBridge\logs\zotero-bridge.log`
- CLI：设 `RUST_LOG=info` 环境变量后运行，日志输出到 stderr。

日志默认只记录库 ID、条目数量、item key 与同步版本，不含标题、摘要
等文献内容（spec 20.1）。
