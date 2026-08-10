# Listary 集成（Windows）

Zotero Bridge 在镜像目录自动维护指向 `zotero://select/...` 的 `.url` 文件
（文件名模板：`{primary_creator} - {year} - {title} -- {item_key}.url`）。

## 设置步骤

1. 运行 `zotero-bridge sync`（日常使用建议 `zotero-bridge sync --watch` 常驻）。
2. 打开 Listary 设置 → 索引/Index，确认目录被收录：
   - 默认：`%LOCALAPPDATA%\ZoteroSearchBridge\mirrors\windows\`
   - 或把 `config.toml` 中 `mirror.windows.directory` 改到你已有的
     索引目录（例如 `D:\Data\ZoteroLinks`）。
3. 在 Listary 中直接输入：
   - `燃气轮机` / `张三 2024` / `Smith turbine` / `N49R8KAQ`
4. 双击 `.url` 结果 → Zotero 打开并定位条目。

文件名包含 作者 + 年份 + 标题 + Item Key，Listary 无需理解 Zotero
数据结构（spec 14.2）。

## 从旧插件迁移

见 spec 第 24 节：新旧目录不要混用。新索引稳定运行后，再从 Listary
索引移除旧导出目录并删除旧 `.url` 文件。`zotero-bridge clean-mirrors` 可清理
与索引不一致的残留文件。
