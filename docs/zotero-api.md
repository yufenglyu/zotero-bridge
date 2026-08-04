# Zotero Local API 实现说明

实现位置：`crates/zotero-api`。

## 使用的端点（只读）

| 端点 | 用途 |
| --- | --- |
| `GET /api/` | 探测；读取 `Zotero-API-Version`、`Zotero-Schema-Version`、`Zotero-Server-ID` 响应头 |
| `GET /api/users/0/groups` | 发现群组库 |
| `GET <prefix>/items/top?since=N&format=versions&includeTrashed=1` | 变化条目版本（分页 100/页） |
| `GET <prefix>/items?itemKey=k1,…,k50&includeTrashed=1` | 批量取条目（≤50 key） |
| `GET <prefix>/deleted?since=N` | 删除记录 |

`<prefix>` 为 `/users/0`（个人库）或 `/groups/<id>`（群组库）。

## 错误语义

- 连接失败/超时 → `ZoteroOffline`（不清空索引，搜索仍可用）
- `403` → Local API 未启用，提示用户在 Zotero 设置中打开
- `412` → 实例不匹配，停止同步并为新实例建立独立分区

## 版本语义与 beta 回退

标准增量依赖对象版本与 `Last-Modified-Version`（spec 12.4 要求各响应
版本一致，否则退避重试）。实测 Zotero `10.0-beta.22` 对以上字段返回
空值（`"version": ""`、`Last-Modified-Version: 0`）。此时引擎自动切换
为全量扫描：

1. `format=versions` 仅用作“全量 key 列表”；
2. 批量拉取全部条目，用 content-hash 跳过未变化条目；
3. 本地有而远端没有的 key 视为已删除；
4. 回收站条目（`deleted: 1`）按删除处理；
5. `last_version` 不推进，下一轮仍是全量扫描。

稳定版 Zotero 返回真实版本号时自动回到标准增量路径，无需配置。
