#!/bin/sh
# 从 1024 母版（margin.svg 和 margin-*.svg）重出打进应用包的图标。改了母版才需要跑；要 rsvg-convert 和 Pillow。
# 主图标是「朱」（margin.svg → AppIcon60x60），别的底色是设置 › 外观里可换的（Info.plist 的 CFBundleAlternateIcons，键就是文件名里的 *）。
# Large*.png 是 276px 的大图，编进二进制，给设置里的小样和关于页用（appicon.rs）。
set -eu
cd "$(dirname -- "$0")"
png() {
    rsvg-convert -w "$2" "$1" -o "$3"
    # 图标不能带透明通道。
    python3 -c "import sys; from PIL import Image; Image.open(sys.argv[1]).convert('RGB').save(sys.argv[1])" "$3"
}
png margin.svg 120 AppIcon60x60@2x.png; png margin.svg 180 AppIcon60x60@3x.png; png margin.svg 276 Large.png
for key in mo molv dian qie zhe wu xuan; do
    name=$(printf '%s' "$key" | awk '{ print toupper(substr($0, 1, 1)) substr($0, 2) }')
    png "margin-$key.svg" 120 "Icon$name@2x.png"; png "margin-$key.svg" 180 "Icon$name@3x.png"; png "margin-$key.svg" 276 "Large$name.png"
done
