#!/bin/sh
# 从三张 1024 母版（mo / molv / zhu .svg）重出打进应用包的图标。改了母版才需要跑；要 rsvg-convert 和 Pillow。
# 主图标是「墨」（AppIcon60x60），另外两张是设置 › 外观里可换的（Info.plist 的 CFBundleAlternateIcons）。
set -eu
cd "$(dirname -- "$0")"
for pair in "mo AppIcon60x60" "molv IconMolv" "zhu IconZhu"; do
    set -- $pair
    for s in 2 3; do
        rsvg-convert -w $((60 * s)) "$1.svg" -o "$2@${s}x.png"
        # 图标不能带透明通道。
        python3 -c "import sys; from PIL import Image; Image.open(sys.argv[1]).convert('RGB').save(sys.argv[1])" "$2@${s}x.png"
    done
done
