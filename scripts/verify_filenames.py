# -*- coding: utf-8 -*-
"""verify_filenames.py — 链接文件名与 Zotero 附件重命名结果逐字比对回归工具。

以 Zotero 实际重命名后的附件文件名（同一命名模板下 Zotero 引擎的真实
输出）为基准，抽查各条目类型，与 zsb 生成的 .url 链接名逐字比对。
用于模板引擎（crates/mirror/src/ztemplate.rs）改动后的回归验证。

前提：
  1. Zotero 正在运行（本地 API 可用，默认 http://127.0.0.1:23119）；
  2. 已执行过同步/刷新链接，镜像目录里有最新 .url 文件。

用法：
  python scripts/verify_filenames.py [--api URL] [--mirror DIR]
                                     [--sample N] [--types a,b,c]

退出码：0 = 全部逐字一致；1 = 存在不一致或环境不可用。

注意：不一致条目需要人工甄别——若 Zotero 附件是在旧模板/旧元数据时期
重命名的，其文件名与当前模板输出不同属正常（可在 Zotero 里重新执行
「从父条目重命名附件」使其一致）。
"""
import argparse
import json
import os
import sys
import urllib.request

DEFAULT_TYPES = [
    "journalArticle", "conferencePaper", "thesis", "preprint",
    "book", "standard", "patent", "report", "document", "dataset",
]


def get_json(url, timeout=15):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def attachment_pdf_stem(api, parent_key):
    """取父条目第一个 PDF 附件的文件名（不含扩展名）。

    children 列表不含 filename，需取完整条目；文件名在 path 字段
    （attachments:/storage: 前缀 + 可能含目录）。
    """
    try:
        children = get_json(f"{api}/items/{parent_key}/children")
    except Exception:
        return None
    for c in children:
        d = c.get("data", {})
        if d.get("itemType") != "attachment" or d.get("contentType") != "application/pdf":
            continue
        try:
            full = get_json(f"{api}/items/{c['key']}")["data"]
        except Exception:
            continue
        path = full.get("path") or ""
        for prefix in ("attachments:", "storage:"):
            if path.startswith(prefix):
                path = path[len(prefix):]
        fname = path.replace("\\", "/").rsplit("/", 1)[-1]
        if fname.lower().endswith(".pdf"):
            return fname[:-4]
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--api", default="http://127.0.0.1:23119/api/users/0",
                    help="Zotero 本地 API 前缀（默认 %(default)s）")
    ap.add_argument("--mirror",
                    default=os.path.expandvars(
                        r"%LOCALAPPDATA%\ZoteroSearchBridge\mirrors\windows"),
                    help=".url 链接目录（默认 %(default)s）")
    ap.add_argument("--sample", type=int, default=6, help="每种类型抽查条数（默认 6）")
    ap.add_argument("--types", default=",".join(DEFAULT_TYPES),
                    help="逗号分隔的条目类型（默认 10 种常见类型）")
    args = ap.parse_args()

    if not os.path.isdir(args.mirror):
        print(f"镜像目录不存在：{args.mirror}", file=sys.stderr)
        return 1
    mirror_files = set(os.listdir(args.mirror))
    if not mirror_files:
        print(f"镜像目录为空：{args.mirror}（先执行同步/刷新链接）", file=sys.stderr)
        return 1

    total = match = mismatch = skipped = 0
    mismatches = []
    for t in [x.strip() for x in args.types.split(",") if x.strip()]:
        try:
            items = get_json(f"{args.api}/items/top?itemType={t}&limit=200")
        except Exception as e:
            print(f"[{t}] API 不可用：{e}", file=sys.stderr)
            return 1
        checked = 0
        for it in items:
            if checked >= args.sample:
                break
            if it.get("meta", {}).get("numChildren", 0) < 1:
                continue
            stem = attachment_pdf_stem(args.api, it["key"])
            if not stem:
                continue
            checked += 1
            total += 1
            if f"{stem}.url" in mirror_files:
                match += 1
                continue
            title = it["data"].get("title", "")
            cand = [f[:-4] for f in mirror_files if title[:12] and title[:12] in f]
            if cand:
                mismatch += 1
                mismatches.append((t, stem, cand[0]))
            else:
                skipped += 1
                print(f"  ? [{t}] 无对应链接（条目未同步？）: {stem[:60]}")

    print()
    print(f"抽查 {total} 条：逐字一致 {match}，不一致 {mismatch}，无对应 {skipped}")
    for t, z, o in mismatches:
        print(f"\n[{t}]")
        print(f"  Zotero: {z}")
        print(f"  zsb   : {o}")
    if mismatches:
        print("\n提示：不一致项请先确认 Zotero 附件是否为当前模板重命名"
              "（旧模板/旧元数据的遗留文件名属正常差异）。")
    return 1 if mismatch else 0


if __name__ == "__main__":
    sys.exit(main())
