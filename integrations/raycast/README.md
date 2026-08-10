# Raycast 扩展

在 Raycast 中创建一个新的 Extension（Template: "Script Command" 或
TypeScript Extension），输入变化时调用：

```sh
zotero-bridge search "<query>" --format json --limit 30
```

返回结构（`SearchResult[]`）：

```json
[
  {
    "item_key": "N49R8KAQ",
    "library_kind": "user",
    "title": "燃气轮机转子动力学研究",
    "creators": "张三; 李四",
    "year": "2024",
    "container_title": "Journal of Turbomachinery",
    "select_uri": "zotero://select/library/items/N49R8KAQ",
    "score": -1.23
  }
]
```

TypeScript 列表示例：

```ts
import { List, ActionPanel, Action, open } from "@raycast/api";
import { useExec } from "@raycast/utils";

type Hit = {
  item_key: string;
  title: string;
  creators: string;
  year: string;
  container_title: string;
  select_uri: string;
};

export default function Command() {
  const { data, isLoading, revalidate } = useExec<Hit[]>("zotero-bridge", ["search", "{query}", "--format", "json"]);
  // 简化示意：把 data 映射为 List.Item
  // 主操作：await open(hit.select_uri)
  // 附加操作：复制标题 / 复制 zotero 链接
  return <List isLoading={isLoading}>{/* ... */}</List>;
}
```

建议操作（spec 15.2）：回车打开 `select_uri`；`Cmd+C` 复制标题；
`Cmd+Shift+C` 复制 Zotero 链接。
