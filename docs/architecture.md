# 文档索引

需求与架构基线由当前 README 与本目录文档共同维护。本目录保存实现过程中补充的专题说明：

- [`zotero-api.md`](zotero-api.md) — Local API 端点、响应头、版本语义与 Zotero 10.0-beta 的兼容性回退
- [`release.md`](release.md) — 构建与发布说明

核心数据流：

```text
Zotero Local API
  -> 增量同步
  -> SQLite FTS5 本地索引
  -> .url / .webloc 链接文件
  -> Listary / Finder / Spotlight
  -> zotero://select/... 定位条目
```
