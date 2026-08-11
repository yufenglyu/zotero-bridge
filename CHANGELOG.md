# Changelog

## 0.12.0

- 完成项目命名统一，清理旧项目名和旧可执行文件名残留。
- 移除 CLI 入口和相关文档，聚焦桌面端同步、诊断与链接文件工作流。
- 优化状态页和设置页布局，新增链接文件目录状态、平台自适应设置与 URI 模板配置。
- 重写 README，补充 Windows 与 macOS 的链接文件使用说明。
- 补充 macOS 本地打包脚本，并复用到 GitHub Actions 发布流程。

## 0.1.0

Zotero Bridge 首个公开版本。

- 桌面端提供状态、设置、诊断和底部状态栏，支持浅色、深色、跟随系统主题。
- 从 Zotero Local API 增量同步文献元数据到本地 SQLite FTS5 索引。
- 支持 Windows `.url` 和 macOS `.webloc` 链接文件，用于 Listary、Finder、Spotlight 等工具定位 Zotero 条目。
- 链接文件支持跟随 Zotero 命名模板、自定义 URI 模板、刷新覆盖、缺失补写和孤儿文件清理。
- 索引排除顶层附件、笔记和批注，避免非文献条目污染搜索结果。
- 提供 Windows 便携包和 Tauri 桌面安装包的自动发布流水线。
