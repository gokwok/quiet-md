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

sh "$here/stage-source.sh"
stage="$CARGO_TARGET_DIR/stage"
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
