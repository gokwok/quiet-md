#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
cd "$repo"
target=aarch64-apple-ios-sim
export CARGO_TARGET_DIR="$repo/target/ios"
export IPHONEOS_DEPLOYMENT_TARGET=17.0
app="$CARGO_TARGET_DIR/Quiet.md Prototype.app"
bundle=com.s1dashu.quietmd.prototype

mkdir -p "$CARGO_TARGET_DIR"
# 先在临时目录拼好源码，再按内容同步到固定的 stage：没变的文件不动，cargo 才能增量编译。
fresh=$(mktemp -d "$CARGO_TARGET_DIR/source.XXXXXX")
trap 'rm -rf "$fresh"' EXIT
stage="$CARGO_TARGET_DIR/stage"
mkdir -p "$fresh/prototypes/ios" "$stage"
cp -R "$repo/src" "$fresh/"
cp "$repo/build.rs" "$fresh/"
cp "$here/Cargo.toml" "$here/Cargo.lock" "$here"/*.rs "$here/EditorSurface.m" "$here/sample.xml" "$here/licences.txt" "$fresh/prototypes/ios/"
cp "$repo/LICENSE" "$fresh/"
patch -s -f -F 0 -p1 -d "$fresh" < "$here/port.patch"
rsync -rlc --delete --exclude /assets --exclude /crates --exclude /packaging --exclude /shared-mock "$fresh/" "$stage/"
for dir in assets crates packaging shared-mock; do [ -e "$stage/$dir" ] || ln -s "$repo/$dir" "$stage/$dir"; done
cargo build --locked --manifest-path "$stage/prototypes/ios/Cargo.toml" --target "$target"
mkdir -p "$app"
cp "$CARGO_TARGET_DIR/$target/debug/quiet_md_ios" "$app/quiet_md_ios"
cp "$here/Info.plist" "$app/Info.plist"
cp "$here"/icons/*.png "$app/"
codesign --force --sign - "$app"
printf '%s\n' "$app"

if [ "$#" -eq 0 ]; then exit 0; fi
device=$1
xcrun simctl install "$device" "$app"
xcrun simctl launch --terminate-running-process "$device" "$bundle"
