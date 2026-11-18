#!/bin/sh
# 用开发身份在本地归档，交由 Xcode 在上传时使用分发身份重新签名。
set -eu
: "${TEAM_ID:?Set TEAM_ID to your Apple Developer team ID}"
: "${BUILD_NUMBER:?Set BUILD_NUMBER to a new positive integer}"
case "$BUILD_NUMBER" in *[!0-9]*|0*) echo "Invalid build number" >&2; exit 2;; esac
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ios=$(cd "$here/.." && pwd)
repo=$(cd "$ios/../.." && pwd)
export CARGO_TARGET_DIR="$repo/target/ios" IPHONEOS_DEPLOYMENT_TARGET=17.0
output="$CARGO_TARGET_DIR/testflight"
archive="$output/Quiet-$BUILD_NUMBER.xcarchive"
mkdir -p "$output"
[ ! -e "$archive" ] || { echo "Archive already exists: $archive" >&2; exit 1; }
work=$(mktemp -d "${TMPDIR:-/tmp}/quiet-archive.XXXXXX")
trap 'rm -rf "$work"' EXIT
sh "$ios/stage-source.sh"
cargo build --release --locked --target aarch64-apple-ios --manifest-path "$CARGO_TARGET_DIR/stage/prototypes/ios/Cargo.toml"
cp "$here/project.yml" "$ios/Info.plist" "$ios/QuietMd.entitlements" "$work/"
plutil -replace CFBundleExecutable -string '$(EXECUTABLE_NAME)' "$work/Info.plist"
plutil -replace CFBundleVersion -string "$BUILD_NUMBER" "$work/Info.plist"
plutil -remove CFBundleIcons "$work/Info.plist"
for name in AppIcon molv zhu; do
    icon="$work/Assets.xcassets/$name.appiconset"
    mkdir -p "$icon"
    source=$name
    [ "$name" != AppIcon ] || source=mo
    rsvg-convert -w 1024 -h 1024 "$ios/icons/$source.svg" -o "$icon/icon.png"
    python3 -c "import sys; from PIL import Image; Image.open(sys.argv[1]).convert('RGB').save(sys.argv[1])" "$icon/icon.png"
    echo '{"images":[{"filename":"icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}],"info":{"author":"xcode","version":1}}' > "$icon/Contents.json"
done
binary="$CARGO_TARGET_DIR/aarch64-apple-ios/release/quiet_md_ios"
cp "$binary" "$work/quiet_md_ios"
cd "$work"
xcodegen generate
set -- DEVELOPMENT_TEAM="$TEAM_ID"
if [ -n "${DEVELOPMENT_PROFILE:-}" ]; then
    set -- "$@" CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY="Apple Development" PROVISIONING_PROFILE_SPECIFIER="$DEVELOPMENT_PROFILE"
fi
xcodebuild -project Quiet.xcodeproj -scheme Quiet -configuration Release -destination 'generic/platform=iOS' \
    -archivePath "$work/Quiet.xcarchive" -derivedDataPath "$work/derived" "$@" archive
app="$work/Quiet.xcarchive/Products/Applications/Quiet.app"
codesign --verify --strict "$app"
# 确认归档里打包了本次 Release 的可执行文件。签名和 strip 会改变文件哈希，但保留 Mach-O UUID。
[ "$(xcrun dwarfdump --uuid "$binary" | cut -d ' ' -f 2)" = "$(xcrun dwarfdump --uuid "$app/quiet_md_ios" | cut -d ' ' -f 2)" ]
mv "$work/Quiet.xcarchive" "$archive"
echo "$archive"
