#!/bin/sh
# 从 target/ios/dev（已改好的 src 和 build.rs）重新生成 port.patch。
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
dev="$repo/target/ios/dev"
[ -d "$dev/src" ] || { echo "先运行 prototypes/ios/dev-tree.sh 建立 $dev" >&2; exit 1; }
work=$(mktemp -d "$repo/target/ios/diff.XXXXXX")
trap 'rm -rf "$work"' EXIT
ln -s "$repo" "$work/a"
ln -s "$dev" "$work/b"
cd "$work"
{ diff -u a/build.rs b/build.rs || true; diff -ruN a/src b/src || true; } \
    | sed -E -e '/^diff -ruN /d' -e 's/^(---|\+\+\+) ([ab]\/[^[:space:]]+)[[:space:]].*$/\1 \2/' > "$here/port.patch"
grep -c '^+++ ' "$here/port.patch"
