#!/bin/sh
# 构建真机包，用开发描述文件签名，给了设备就装上并启动。
# 描述文件和设备只从参数、环境变量读，不写进仓库：
#   QUIET_MD_IOS_PROFILE  开发描述文件（.mobileprovision）的路径，必填
#   QUIET_MD_IOS_DEVICE   设备的 UDID 或名称（`xcrun devicectl list devices`），可用第一个参数代替；都不给时只构建并签名
#   DEBUG=1               构建 debug 包（默认 release；debug 在真机上明显发涩）
#   BUILD_NUMBER=<n>      写进包里的 CFBundleVersion（默认用 Info.plist 的）
#   CONSOLE=1             启动后接上应用的输出，直到应用退出
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
cd "$repo"
profile=${QUIET_MD_IOS_PROFILE:?设 QUIET_MD_IOS_PROFILE 为开发描述文件的路径}
device=${1:-${QUIET_MD_IOS_DEVICE:-}}
target=aarch64-apple-ios
export CARGO_TARGET_DIR="$repo/target/ios"
export IPHONEOS_DEPLOYMENT_TARGET=17.0
app="$CARGO_TARGET_DIR/device/Quiet.md Prototype.app"
bundle=$(plutil -extract CFBundleIdentifier raw "$here/Info.plist")
build=release
[ "${DEBUG:-}" != 1 ] || build=debug
if [ -n "${BUILD_NUMBER:-}" ]; then
    case "$BUILD_NUMBER" in *[!0-9]*|0*) echo "Invalid build number" >&2; exit 2;; esac
fi

sh "$here/stage-source.sh"
# Unsplash Access Key 不进仓库：放在 prototypes/ios/unsplash.key（已忽略），构建时编进二进制。
[ ! -f "$here/unsplash.key" ] || export UNSPLASH_ACCESS_KEY="$(tr -d '[:space:]' < "$here/unsplash.key")"
if [ "$build" = release ]; then set -- --release; else set --; fi
cargo build "$@" --locked --manifest-path "$CARGO_TARGET_DIR/stage/prototypes/ios/Cargo.toml" --target "$target"
rm -rf "$app"
mkdir -p "$app"
cp "$CARGO_TARGET_DIR/$target/$build/quiet_md_ios" "$app/quiet_md_ios"
cp "$here/Info.plist" "$app/Info.plist"
[ -z "${BUILD_NUMBER:-}" ] || plutil -replace CFBundleVersion -string "$BUILD_NUMBER" "$app/Info.plist"
cp "$here"/icons/AppIcon*.png "$here"/icons/Icon*.png "$app/"
python3 "$here/sign-device.py" "$profile" --app "$app"
printf '%s\n' "$app"

if [ -z "$device" ]; then exit 0; fi
xcrun devicectl device install app --device "$device" "$app"
if [ "${CONSOLE:-}" = 1 ]; then
    xcrun devicectl device process launch --device "$device" --terminate-existing --console "$bundle"
else
    xcrun devicectl device process launch --device "$device" --terminate-existing "$bundle"
fi
