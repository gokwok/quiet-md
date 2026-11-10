#!/bin/sh
# 建立可直接编辑的 target/ios/dev：当前桌面源码加上 port.patch。已存在时不覆盖。
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
dev="$repo/target/ios/dev"
[ ! -e "$dev" ] || { echo "$dev 已存在；确认里面的改动已写回 port.patch 后先删掉它" >&2; exit 1; }
mkdir -p "$dev"
cp -R "$repo/src" "$dev/"
cp "$repo/build.rs" "$dev/"
patch -s -f -F 0 -p1 -d "$dev" < "$here/port.patch"
printf '%s\n' "$dev"
