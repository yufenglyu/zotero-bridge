# Changelog

## 0.1.0

Zotero Bridge 首个公开版本。

- 桌面端提供状态、设置、诊断和底部状态栏，支持浅色、深色、跟随系统主题。
- 从 Zotero Local API 增量同步文献元数据到本地 SQLite FTS5 索引。
- 支持 Windows `.url` 和 macOS `.webloc` 链接文件，用于 Listary、Alfred、Raycast 等启动器定位 Zotero 条目。
- 链接文件支持跟随 Zotero 命名模板、自定义 URI 模板、刷新覆盖、缺失补写和孤儿文件清理。
- 索引排除顶层附件、笔记和批注，避免非文献条目污染搜索结果。
- 提供 Windows 便携包和 Tauri 桌面安装包的自动发布流水线。
