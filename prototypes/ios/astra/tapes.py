"""Astra 的十二款分隔胶带。单个花回首尾相接，输出 800×192 的印花纸纹。"""
from io import BytesIO
from pathlib import Path
import subprocess

import numpy as np
from PIL import Image

WASHI = Path(__file__).resolve().parent / 'washi'


def tape(name, ground, period, motif, defs=''):
    # 一个花回覆盖整个胶带高度；相邻花回也参与绘制，跨边缘的笔画不会被截断。
    width, height = 400, 96
    repeats = ''.join(f'<g transform="translate({x} 0)">{motif}</g>' for x in range(-period, width + period, period))
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="800" height="192" viewBox="0 0 {width} {height}">
        <defs>{defs}</defs><rect width="400" height="96" fill="{ground}"/>{repeats}</svg>'''
    raster = subprocess.run(['rsvg-convert'], input=svg.encode(), capture_output=True, check=True)
    pixels = np.array(Image.open(BytesIO(raster.stdout)).convert('RGB'), dtype=np.float32)
    rng = np.random.default_rng(sum(map(ord, name)))
    grain = rng.normal(0, 2.2, (192, 800, 1))
    pixels += grain
    out = Image.fromarray(np.clip(pixels, 0, 255).astype(np.uint8))
    out.save(WASHI / f'{name}.jpg', quality=90, optimize=True, subsampling=0)


def dots(points, color):
    return ''.join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{color}"/>' for x, y, r in points)


def main():
    WASHI.mkdir(exist_ok=True)
    # 鲸背天文台 · 星图经线、金箔星芒与月相。
    tape('skywhale', '#122a43', 200, '''
        <g stroke="#b99152" fill="none" stroke-width=".9">
        <path d="M0 8H200 M0 88H200 M0 80Q95 -37 200 76 M0 61L31 40 49 48 81 21 139 45 170 26 200 61"/>
        <ellipse cx="107" cy="52" rx="43" ry="29" transform="rotate(-23 107 52)"/></g>
        <circle cx="107" cy="52" r="19" fill="#ebc982"/><circle cx="114" cy="47" r="17" fill="#122a43"/>
        <path d="M31 29l2 9 8 2 -8 2 -2 11 -2 -11 -8 -2 8 -2z M168 47l2 8 8 2 -8 2 -2 9 -2 -9 -8 -2 8 -2z" fill="#f9e7bd"/>
        ''' + dots([(49,48,2),(81,21,2),(139,45,2),(170,26,1.6),(62,72,1.3),(188,17,1.5)], '#edd59d'))
    # 柑橘邮局 · 航空信封斜纹、橙子剖面与燕尾形绿叶。
    tape('citrus', '#ffefbc', 100, '''
        <path d="M-10 12L2 0h16L6 12z M40 12L52 0h16L56 12z M-10 96l12-12h16L6 96z M40 96l12-12h16L56 96z" fill="#e84b28"/>
        <path d="M15 12L27 0h12L27 12z M65 12L77 0h12L77 12z M15 96l12-12h12L27 96z M65 96l12-12h12L77 96z" fill="#315c48"/>
        <circle cx="37" cy="48" r="27" fill="#ef7826"/><circle cx="37" cy="48" r="22" fill="#e84522" stroke="#ffdda1" stroke-width="2"/>
        <path d="M37 27v42 M16 48h42 M22 33l30 30 M22 63l30-30" stroke="#ffdfad" stroke-width="1.8"/>
        <path d="M62 49Q66 19 94 21Q91 45 62 49 M64 53q26-2 28 22-23-1-28-22" fill="#24583e"/>
        <path d="M64 47L86 26m-18 30 16 14" stroke="#9cada0" fill="none"/>''')
    # 冰川折页 · 一条完整的折线把亮面与阴影缝在一起。
    tape('glacier', '#addcd4', 200, '''
        <circle cx="158" cy="20" r="13" fill="#fb7e3c"/>
        <path d="M-12 87L43 18 64 45 80 32 139 93Z" fill="#f0fbdf"/>
        <path d="M43 18L22 85 72 60Z" fill="#608fbd"/><path d="M43 18l21 27 16-13-8 28z" fill="#d5eee3"/>
        <path d="M53 96l66-75 32 38 24-13 38 50Z" fill="#caeae6"/><path d="M119 21L88 96h65l-18-39z" fill="#3e6cad"/>
        <path d="M119 21l16 36 16 2z" fill="#f3f5dc"/><path d="M0 77l34-22 66 41H0zm121 19 57-41 30 29v12z" fill="#245184"/>''')
    # 胭脂放映室 · 胶片方孔、遮幅与银幕里的弯月。
    holes = ''.join(f'<rect x="{i}" y="{y}" width="9" height="5" rx="1.5" fill="#e4c39d"/>' for i in range(4, 100, 16) for y in (5, 86))
    tape('matinee', '#3b1524', 100, holes + '''
        <rect x="4" y="17" width="92" height="61" rx="3" fill="#c95055"/>
        <path d="M4 17h92v6H4z M4 17q28 42 7 61H4z M96 17Q68 59 89 78h7z" fill="#84253d"/>
        <path d="M56 28a18 18 0 1 0 12 28A17 17 0 0 1 56 28Z" fill="#f5d69a"/>
        <path d="M17 73h66m-60-4h54" stroke="#f0a889" fill="none"/>''')
    # 月面修理铺 · 卡尺刻度与齿轮模数线。
    ticks = ''.join(f'<path d="M{x} 0v{12 if x % 20 == 0 else 6}M{x} 96v-{12 if x % 20 == 0 else 6}"/>' for x in range(0, 100, 5))
    teeth = ''.join(f'<rect x="46.5" y="17" width="7" height="9" transform="rotate({a} 50 48)"/>' for a in range(0, 360, 30))
    tape('lunar', '#eeb749', 100, f'''
        <g stroke="#27291f" fill="none" stroke-width="1.5">{ticks}<path d="M0 48H100 M50 12v72" stroke-dasharray="6 3 1 3"/>
        <path d="M0 20h13l8 11 M79 65l8 11h13"/></g>
        <g fill="#2a2c24">{teeth}<circle cx="50" cy="48" r="25"/></g>
        <circle cx="50" cy="48" r="18" fill="#eeb749" stroke="#f8e5a5" stroke-width="2"/>
        <circle cx="50" cy="48" r="9" fill="#34362b"/><circle cx="50" cy="48" r="4" fill="#f8e5a5"/>''')
    # 霓虹雨巷 · 每面窗户和雨伞都沿同一像素网格绘制。
    windows = ''.join(f'<rect x="{x}" y="{y}" width="4" height="6" fill="{c}"/>' for x, y, c in [(4,27,'#9081ee'),(14,27,'#8accdc'),(4,45,'#8accdc'),(14,45,'#655aa0'),(38,19,'#7cb9ec'),(48,19,'#a16ddc'),(38,34,'#564c8b'),(80,31,'#8accdc'),(90,31,'#9868d5')])
    tape('rain', '#544e91', 100, '''
        <path d="M0 20h25v35h7V11h28v44h10V24h30v52H0z" fill="#272d51"/>
        <path d="M0 72h100v24H0z" fill="#312d5c"/>
        <path d="M32 15h28 M0 60h22 M70 56h30 M5 81h21 M74 88h23" stroke="#76e2da" stroke-width="2"/>''' + windows + '''
        <path d="M42 62v-4h4v-4h6v-4h10v4h6v4h4v4z" fill="#fa514e"/>
        <path d="M56 62v12h-4" fill="none" stroke="#ecaaac" stroke-width="2"/>
        <path d="M44 80h23m-17 4h12" stroke="#df667a" stroke-width="2"/>''')
    # 孔雀花窗 · 每个眼斑都是一小块铅条围合的彩玻。
    tape('peacock', '#156356', 100, '''
        <g stroke="#203a3b" stroke-width="3">
        <path d="M0 91Q22 79 50 6Q78 79 100 91Q46 119 0 91Z" fill="#328476"/>
        <path d="M50 82C15 77 25 47 50 16C75 47 85 77 50 82Z" fill="#daa44c"/>
        <path d="M50 73C30 70 34 50 50 32C69 54 70 68 50 73Z" fill="#4cb1a4"/>
        <path d="M50 65C38 64 39 54 50 45C62 54 60 64 50 65Z" fill="#28487d"/>
        <path d="M0 0q0 34 24 50Q17 22 50 0 M100 0q0 34-24 50Q83 22 50 0" fill="#236898"/></g>''')
    # 沙海驿站 · 拱券连续穿过沙丘，铁道和车窗是两道细长的横线。
    tape('dune', '#e8ab70', 200, '''
        <circle cx="51" cy="21" r="15" fill="#dd583e"/>
        <path d="M0 67q44-45 106-23t94-8v60H0z" fill="#cf8058"/>
        <path d="M0 69q59-26 125-11t75-2v40H0z" fill="#a85542"/>
        <path d="M0 43h200v53h-9V70a16 16 0 0 0-32 0v26h-18V70a16 16 0 0 0-32 0v26H91V70a16 16 0 0 0-32 0v26H41V70a16 16 0 0 0-32 0v26H0z" fill="#673f3c"/>
        <path d="M0 38h117q14 0 22 5H0zm149 0h51v5h-51z" fill="#fff0c7"/><path d="M0 45h200" stroke="#e7bd8d" stroke-width="2"/>''')
    # 云锦来信 · 锯齿云纹、飞鹤和贯穿布面的经纬。
    yarn = ''.join(f'<path d="M{x} 0v96"/>' for x in range(0, 100, 3)) + ''.join(f'<path d="M0 {y}h100"/>' for y in range(0, 96, 4))
    tape('brocade', '#843552', 100, '''
        <path d="M0 5h15v5h10v5h25v-5h10V5h20v6h20v8H78v-5H62v6H49v4H22v-5H11v-6H0z M0 76h16v-8h19v5h12v7h28v-6h25v9H78v7H43v-7H26v-5h-8v8H0z" fill="#ec9167"/>
        <path d="M5 52h14v-4h14l-8-13-11-12 18 7 14 10 10 6 14-3 9-10h10l-9 5-4 9-15 9H40l-17 9 4-9H5z" fill="#ffdfb1"/>
        <path d="M15 23l13 4 11 8-10-2z M38 56h15l-19 9z" fill="#3d4453"/><path d="M80 32h6v4h-6z" fill="#f57550"/>
        <g stroke="#4f3047" opacity=".19" stroke-width=".7">''' + yarn + '</g>')
    # 桃色引力 · 冷暖反射在镀铬圆环上压成几道高光。
    tape('gravity', '#dfc4db', 100, '''
        <ellipse cx="50" cy="74" rx="31" ry="4" fill="#baa3c0"/>
        <g transform="rotate(-32 49 45)"><ellipse cx="49" cy="45" rx="29" ry="20" fill="none" stroke="url(#chrome)" stroke-width="13"/></g>
        <circle cx="50" cy="40" r="10" fill="url(#pearl)"/><circle cx="92" cy="63" r="5" fill="url(#pearl)"/>''',
        '''<linearGradient id="chrome" x1="0" x2=".3" y1="0" y2="1" gradientUnits="objectBoundingBox">
        <stop stop-color="#fff0de"/><stop offset=".24" stop-color="#e9fff1"/><stop offset=".44" stop-color="#656c78"/><stop offset=".48" stop-color="#273b48"/><stop offset=".56" stop-color="#9fcfc4"/><stop offset=".74" stop-color="#e6f2e0"/><stop offset="1" stop-color="#957a95"/></linearGradient>
        <radialGradient id="pearl" cx=".3" cy=".25" r=".8"><stop stop-color="#fff0c8"/><stop offset=".5" stop-color="#eea488"/><stop offset="1" stop-color="#9b6585"/></radialGradient>''')
    # 夜航水母 · 铃状伞体的放射纹和弯曲的游丝。
    filaments = ''.join(f'<path d="M{x} {42 + (x % 3)}q{9-x%7} 12 -2 23t5 {12+x%5}"/>' for x in range(24, 67, 6))
    tape('medusae', '#153b3d', 100, '''
        <path d="M15 46C17 7 60 3 73 41Q66 50 59 44Q52 53 45 46Q36 53 31 46Q21 51 15 46Z" fill="#e9ab98" stroke="#f7cfb4" stroke-width="1.4"/>
        <g fill="none" stroke="#965f66"><path d="M42 16Q23 26 20 45 M42 16q-8 12-11 29 M43 16q3 13 2 29 M44 16q14 11 15 27 M44 16q19 4 24 26"/></g>
        <g fill="none" stroke="#e7b3a6" stroke-width="1.3">''' + filaments + '</g>' + dots([(6,18,1),(89,38,1.7),(79,72,1),(12,80,1.6),(91,11,1)], '#93c9b6'))
    # 无尽阶梯 · 两组并排的台阶错半层，只有朱红小球离开灰阶。
    tape('stairs', '#d3cfc1', 100, '''
        <path d="M0 58l50-28 50 28-50 29Z" fill="#666962"/>
        <g fill="#f6f0dd" stroke="#444744" stroke-width=".7">
        <path d="M6 53l39-22 13 7-39 23z M16 60l39-23 12 7-39 23z M26 67l39-23 12 7-39 23z M36 74l39-23 12 7-39 23z"/>
        </g><path d="M6 53l13 8v7L6 60zm10 7 12 7v7l-12-7zm10 7 12 7v7l-12-7zm10 7 12 7v7l-12-7z" fill="#50544f"/>
        <circle cx="45" cy="26" r="9" fill="#d84433"/><circle cx="42" cy="23" r="1.8" fill="#f07757"/>''')


if __name__ == '__main__':
    main()
