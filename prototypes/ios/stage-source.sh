#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
CARGO_TARGET_DIR="$repo/target/ios"
mkdir -p "$CARGO_TARGET_DIR"
# 先在临时目录拼好源码，再按内容同步到固定的 stage：没变的文件不动，cargo 才能增量编译。
fresh=$(mktemp -d "$CARGO_TARGET_DIR/source.XXXXXX")
trap 'rm -rf "$fresh"' EXIT
stage="$CARGO_TARGET_DIR/stage"
mkdir -p "$fresh/prototypes/ios" "$stage"
cp -R "$repo/src" "$fresh/"
cp "$repo/build.rs" "$fresh/"
cp "$here/Cargo.toml" "$here/Cargo.lock" "$here"/*.rs "$here/EditorSurface.m" "$here/sample.xml" "$here/licences.txt" "$fresh/prototypes/ios/"
cp -R "$here/avatars" "$here/icons" "$here/washi" "$here/backdrops" "$here/claude" "$here/astra" "$fresh/prototypes/ios/"
cp "$repo/LICENSE" "$fresh/"
patch -s -f -F 0 -p1 -d "$fresh" < "$here/port.patch"
rsync -rlc --delete --exclude /assets --exclude /crates --exclude /packaging --exclude /shared-mock "$fresh/" "$stage/"
for dir in assets crates packaging shared-mock; do [ -e "$stage/$dir" ] || ln -s "$repo/$dir" "$stage/$dir"; done
