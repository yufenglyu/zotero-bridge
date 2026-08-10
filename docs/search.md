# 搜索实现说明

实现位置：`crates/index/src/search.rs`、migrations/0002_fts.sql。

## 索引

FTS5 外部内容表 `items_fts`，trigram tokenizer（`case_sensitive 0`），
通过 insert/update/delete 三个触发器与 `items` 保持一致（spec 9.1–9.2）。

## 查询策略

| 词长（Unicode 字符数） | 策略 |
| --- | --- |
| ≥ 3 | FTS5 `MATCH`，短语加双引号，多词 AND |
| 1–2 | `LIKE '%…%'` 后置过滤；整句均为短词时纯 LIKE 回退（spec 9.3） |

字段限定（spec 9.5 增强版）：`author:` / `year:` / `tag:` / `type:` /
`library:`。长值走列限定 FTS（如 `{primary_creator creators} : "smith"`），
短值走 LIKE。所有值均参数绑定，FTS 短语内的 `"` 双写转义，LIKE 模式
转义 `\`、`%`、`_`（`ESCAPE '\'`）。

参数绑定顺序固定为：MATCH 参数 → 全部 LIKE 条件 → 全部等值条件 →
LIMIT（与 SQL 拼装顺序一致，勿按词序绑定）。

## 排序

`bm25(items_fts, 10.0, 6.0, 4.0, 3.0, 2.5, 2.0, 0.5, 0.5)` 升序
（FTS5 BM25 越小越好），权重对应：标题 / 第一作者 / 全部作者 / 年份 /
出版物 / 标签 / 摘要 / Extra（spec 9.4）。LIKE 回退路径按年份、标题排序。

## 维护

- `zotero-bridge rebuild` → `INSERT INTO items_fts(items_fts) VALUES('rebuild')`
- `zotero-bridge verify-index` → `VALUES('integrity-check', 1)`
- `zotero-bridge optimize` → `VALUES('optimize')`
