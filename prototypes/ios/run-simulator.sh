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
# RELEASE=1 构建 release 包：看真实的流畅度用（debug 没有优化，换页和滚动会发涩）。
build=debug
[ "${RELEASE:-}" != 1 ] || build=release

sh "$here/stage-source.sh"
stage="$CARGO_TARGET_DIR/stage"
# Unsplash Access Key 不进仓库：放在 prototypes/ios/unsplash.key（已忽略），构建时编进二进制。
[ ! -f "$here/unsplash.key" ] || export UNSPLASH_ACCESS_KEY="$(tr -d '[:space:]' < "$here/unsplash.key")"
if [ "$build" = release ]; then
    cargo build --release --locked --manifest-path "$stage/prototypes/ios/Cargo.toml" --target "$target"
else
    cargo build --locked --manifest-path "$stage/prototypes/ios/Cargo.toml" --target "$target"
fi
mkdir -p "$app"
cp "$CARGO_TARGET_DIR/$target/$build/quiet_md_ios" "$app/quiet_md_ios"
cp "$here/Info.plist" "$app/Info.plist"
cp "$here"/icons/AppIcon*.png "$here"/icons/Icon*.png "$app/"
codesign --force --sign - "$app"
printf '%s\n' "$app"

if [ "$#" -eq 0 ]; then exit 0; fi
device=$1
xcrun simctl install "$device" "$app"
xcrun simctl launch --terminate-running-process "$device" "$bundle"
