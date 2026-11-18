#!/bin/sh
# Xcode 不负责编译 Rust；验证归档携带的符号来自同一次链接，且包含源码行表。
set -eu
archive=${1:?Usage: verify-symbols.sh path/to/Quiet.xcarchive}
binary="$archive/Products/Applications/Quiet.app/quiet_md_ios"
symbols="$archive/dSYMs/Quiet.app.dSYM/Contents/Resources/DWARF/quiet_md_ios"
[ -s "$symbols" ] || { echo "Missing Rust dSYM: $symbols" >&2; exit 1; }
uuid() { xcrun dwarfdump --uuid "$1" | awk '$1 == "UUID:" && $3 == "(arm64)" { print $2 }'; }
binary_uuid=$(uuid "$binary")
[ -n "$binary_uuid" ] && [ "$binary_uuid" = "$(uuid "$symbols")" ] || { echo "App and dSYM UUIDs do not match" >&2; exit 1; }
xcrun size -m "$symbols" | awk '/Section __debug_line: [1-9]/ { found=1 } END { exit !found }' || { echo "Missing DWARF line table" >&2; exit 1; }
