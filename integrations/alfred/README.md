# Alfred 集成（Script Filter）

1. 确保 `zsb` 在 PATH 中（或使用绝对路径）。
2. 新建 Alfred Workflow，添加 **Script Filter**：
   - Keyword: `z`
   - Language: `/bin/bash`
   - Script:

     ```bash
     /usr/local/bin/zsb search "$1" --format alfred --limit 30
     ```

   - 勾选 "Alfred filters results" 关闭（结果已由 zsb 排序）。
3. 连接一个 **Open URL** Action：`{query}`（zsb 输出的 `arg` 即
   `zotero://select/...` 链接，系统会唤起 Zotero 并定位条目）。

输出格式示例（spec 15.1）：

```json
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

Zotero 未运行时搜索仍可用（读本地索引），回车时系统会启动 Zotero。
