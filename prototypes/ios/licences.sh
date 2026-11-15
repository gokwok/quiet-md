#!/bin/sh
# 重新生成 licences.txt：iOS 版真正编进去的开源组件（只算运行时依赖，按 Cargo.lock 和 aarch64-apple-ios 平台），
# 设置 › 关于 › 开源许可 里列的就是这份。改了 prototypes/ios/Cargo.lock 后跑一遍。每行：包名<TAB>版本<TAB>许可。
set -e
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cargo metadata --locked --format-version 1 --manifest-path "$here/Cargo.toml" --filter-platform aarch64-apple-ios | python3 -c '
import json, sys
m = json.load(sys.stdin)
ws = set(m["workspace_members"])
nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
seen, todo = set(), list(ws)
while todo:
    i = todo.pop()
    if i in seen: continue
    seen.add(i)
    todo += [d["pkg"] for d in nodes[i]["deps"] if any(k.get("kind") is None for k in d["dep_kinds"])]
own = {"fdl_core", "quiet_md", "quiet_md_ios"}
pkgs = sorted((p for p in m["packages"] if p["id"] in seen and p["id"] not in ws and p["name"] not in own), key=lambda p: (p["name"].lower(), p["version"]))
sys.stdout.write("".join(p["name"] + "\t" + p["version"] + "\t" + (p.get("license") or "—") + "\n" for p in pkgs))
' > "$here/licences.txt"
printf 'licences.txt: %s packages\n' "$(wc -l < "$here/licences.txt" | tr -d ' ')"
