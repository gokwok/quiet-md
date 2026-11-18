#!/bin/sh
# 使用 Xcode 已登录的归档所属团队，保留构建号并只分发到 TestFlight 内部组。
set -eu
archive=${1:?Usage: upload.sh path/to/Quiet.xcarchive}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
sh "$here/verify-symbols.sh" "$archive"
team=$(plutil -extract ApplicationProperties.Team raw "$archive/Info.plist")
work=$(mktemp -d "${TMPDIR:-/tmp}/quiet-upload.XXXXXX")
trap 'rm -rf "$work"' EXIT
options="$work/ExportOptions.plist"
plutil -create xml1 "$options"
plutil -insert method -string app-store-connect "$options"
plutil -insert destination -string upload "$options"
plutil -insert signingStyle -string automatic "$options"
plutil -insert teamID -string "$team" "$options"
plutil -insert uploadSymbols -bool YES "$options"
plutil -insert testFlightInternalTestingOnly -bool YES "$options"
plutil -insert manageAppVersionAndBuildNumber -bool NO "$options"
xcodebuild -exportArchive -archivePath "$archive" -exportPath "$work/export" \
    -exportOptionsPlist "$options" -allowProvisioningUpdates
