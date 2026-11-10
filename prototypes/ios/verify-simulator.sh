#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
device=${1:?传入专用的 iPhone 18 Pro 模拟器 UDID}
bundle=com.s1dashu.quietmd.prototype
work=$(mktemp -d /tmp/quiet-md-ios-check.XXXXXX)

"$here/run-simulator.sh"
mkdir "$work/Host" "$work/Smoke"
cp "$here/smoke/Placeholder.swift" "$work/Host/"
cp "$here/smoke/EditorSmoke.swift" "$work/Smoke/"
cp "$here/smoke/project.yml" "$here/Info.plist" "$work/"

cd "$work"
xcodegen generate
project=QuietPrototypeSmoke.xcodeproj
if ! xcodebuild -project "$project" -scheme Smoke -destination "id=$device" \
    -derivedDataPath "$work/DerivedData" build-for-testing > "$work/build.log" 2>&1; then
    tail -70 "$work/build.log"
    exit 1
fi
app="$work/DerivedData/Build/Products/Debug-iphonesimulator/Prototype.app"
cp "$repo/target/ios/Quiet.md Prototype.app/quiet_md_ios" "$app/quiet_md_ios"
codesign --force --sign - "$app"
# Only configure the simulator explicitly selected for the smoke test.
xcrun simctl spawn "$device" defaults write -g AppleKeyboards -array \
    'zh_Hans-Pinyin@sw=Pinyin-Simplified' 'en_US' 'emoji'
touch before
if ! xcodebuild -project "$project" -scheme Smoke \
    -destination "id=$device" -derivedDataPath "$work/DerivedData" \
    -resultBundlePath "$work/Result.xcresult" -parallel-testing-enabled NO test-without-building > "$work/test.log" 2>&1; then
    tail -70 "$work/test.log"
    exit 1
fi

container=$(xcrun simctl get_app_container "$device" "$bundle" data)
library=$(python3 - "$container/tmp" "$work/before" <<'PY'
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

tmp, before = map(Path, sys.argv[1:])
libraries = [p for p in tmp.glob("Quiet.md-prototype-check-*") if p.stat().st_mtime_ns >= before.stat().st_mtime_ns]
if len(libraries) != 1:
    raise SystemExit(f"Expected one new verification library, got {len(libraries)}")
notes = list((libraries[0] / "docs").glob("*.xml"))
if len(notes) != 1:
    raise SystemExit(f"Expected one saved note, got {len(notes)}")
note = ET.fromstring("<doc>" + notes[0].read_text(encoding="utf-8") + "</doc>")
assert note.findtext("p") == "触屏选区：双击选择 你好，长按拖动选择中文。"
assert note.find(".//td[@colspan='2']/p").text == "合并单元格"
assert [el.text for el in note.findall(".//latex")] == [
    r"S=\pi r^2", r"\displaystyle\frac{1}{2}+\sqrt{x^2+1}",
]
print(libraries[0])
PY
)

SIMCTL_CHILD_QUIET_MD_HOME="$library" SIMCTL_CHILD_QUIET_MD_IOS_SAMPLE=1 xcrun simctl launch --terminate-running-process "$device" "$bundle"
sleep 2
xcrun simctl io "$device" screenshot "$work/reopened.png"
printf '测试通过，DocxXML 已在后台写入。\n结果：%s/Result.xcresult\n重启截图：%s/reopened.png\n' "$work" "$work"
