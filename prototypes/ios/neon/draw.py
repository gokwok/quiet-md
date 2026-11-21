#!/usr/bin/env python3
"""Neon 的十二套文档样式（neon.rs）用到的背景、胶带和封面：全部由本脚本程序化绘制（本项目绘制）。

    python3 prototypes/ios/neon/draw.py              # 全部生成并输出 top 色值与对比度
    python3 prototypes/ios/neon/draw.py prism amber  # 只重画指定几套

背景 1066×1600 放 backdrops/，胶带 800×192 放 washi/（花纹周期整除 800，左右无缝衔接），封面 2000×400 放 covers/。
随机数均采用固定种子，重画结果完全一致。
"""
import io
import math
import os
import subprocess
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
BG, TAPE, COVER = (1066, 1600), (800, 192), (2000, 400)


def rgb(h):
    h = h.lstrip('#') if isinstance(h, str) else f'{h:06x}'
    return np.array([int(h[i:i + 2], 16) for i in (0, 2, 4)], dtype=np.float32)


def canvas(size, color):
    w, h = size
    return np.broadcast_to(rgb(color), (h, w, 3)).astype(np.float32).copy()


def grid(size):
    w, h = size
    ys, xs = np.mgrid[0:h, 0:w].astype(np.float32)
    return xs, ys


def ramp(t, stops):
    t = np.clip(t, 0.0, 1.0)
    out = np.zeros(t.shape + (3,), np.float32)
    pos = [p for p, _ in stops]
    cols = [rgb(c) for _, c in stops]
    for ch in range(3):
        out[..., ch] = np.interp(t, pos, [c[ch] for c in cols])
    return out


def blend(img, color, alpha):
    a = np.clip(np.asarray(alpha, np.float32), 0.0, 1.0)
    if a.ndim == 2:
        a = a[..., None]
    c = rgb(color) if isinstance(color, (str, int)) else color
    return img * (1.0 - a) + c * a


def screen(img, color, alpha):
    a = np.clip(np.asarray(alpha, np.float32), 0.0, 1.0)
    if a.ndim == 2:
        a = a[..., None]
    c = rgb(color) if isinstance(color, (str, int)) else color
    s = 255.0 - (255.0 - img) * (255.0 - c) / 255.0
    return img * (1.0 - a) + s * a


def glow(size, cx, cy, r, power=2.0):
    xs, ys = grid(size)
    d = np.hypot(xs - cx, ys - cy) / r
    return np.clip(1.0 - d, 0.0, 1.0) ** power


def grain(img, amount, seed=1, mono=True):
    rng = np.random.default_rng(seed)
    h, w, _ = img.shape
    n = rng.normal(0.0, amount, (h, w, 1) if mono else (h, w, 3)).astype(np.float32)
    return img + n


def noise(size, scale, seed=0, octaves=4):
    w, h = size
    rng = np.random.default_rng(seed)
    out = np.zeros((h, w), np.float32)
    amp, total = 1.0, 0.0
    for o in range(octaves):
        s = max(2.0, scale / (2 ** o))
        gw, gh = int(w / s) + 3, int(h / s) + 3
        g = Image.fromarray((rng.random((gh, gw)) * 255).astype(np.uint8))
        g = g.resize((int(gw * s), int(gh * s)), Image.BICUBIC).crop((0, 0, w, h))
        out += np.asarray(g, np.float32) / 255.0 * amp
        total += amp
        amp *= 0.5
    return out / total


def blur_arr(arr, radius):
    im = Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8))
    return np.asarray(im.filter(ImageFilter.GaussianBlur(radius)), np.float32)


def svg_rgba(markup, size, viewbox=None):
    w, h = size
    vb = viewbox or f'0 0 {w} {h}'
    doc = f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{vb}">{markup}</svg>'
    png = subprocess.run(['rsvg-convert', '-w', str(w), '-h', str(h)], input=doc.encode(), capture_output=True, check=True).stdout
    return np.asarray(Image.open(io.BytesIO(png)).convert('RGBA'), np.float32)


def over(img, rgba):
    a = rgba[..., 3:4] / 255.0
    return img * (1.0 - a) + rgba[..., :3] * a


def tape_svg(ground, period, motif, defs='', grain_amt=2.2, seed=1):
    w, h = 400, 96
    repeats = ''.join(f'<g transform="translate({x} 0)">{motif}</g>' for x in range(-period, w + period, period))
    rgba = svg_rgba(f'<defs>{defs}</defs><rect width="{w}" height="{h}" fill="{ground}"/>{repeats}', TAPE, f'0 0 {w} {h}')
    return grain(rgba[..., :3], grain_amt, seed)


def save(img, kind, name, quality=90):
    folder = {'bg': 'backdrops', 'tape': 'washi', 'cover': 'covers'}[kind]
    out_dir = os.path.join(HERE, folder)
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, f'{name}.jpg')
    clipped = np.clip(img, 0, 255).astype(np.uint8)
    Image.fromarray(clipped).save(path, quality=quality, optimize=True, progressive=True, subsampling=0 if kind == 'tape' else 2)
    return clipped


def top_hex(arr):
    h = max(1, arr.shape[0] // 7)
    m = arr[:h].mean(axis=(0, 1))
    return (int(round(m[0])) << 16) | (int(round(m[1])) << 8) | int(round(m[2]))


def wcag_contrast(c1, c2):
    def lum(h):
        c = rgb(h) / 255.0
        lin = np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
        return float(0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2])
    a, b = lum(c1), lum(c2)
    return (max(a, b) + 0.05) / (min(a, b) + 0.05)


STYLES = {}


def style(fn):
    STYLES[fn.__name__] = fn
    return fn


# ---------------------------------------------------------------- 1. 棱镜色散 (Prism)
@style
def prism():
    # 背景：暗室黑曜底，斜向光学棱镜色散光束 + 微弱二次折射
    xs, ys = grid(BG)
    w, h = BG
    img = ramp(ys / h, [(0.0, '#0B0D14'), (0.5, '#10131F'), (1.0, '#080A10')])
    # 主光束坐标系
    u = (xs - 533.0) / 533.0
    v = (ys - 760.0) / 800.0
    # 入射白光（左上斜入）
    beam_in = np.exp(-((0.52 * xs - ys + 180.0) / 55.0) ** 2) * np.clip((580.0 - xs) / 260.0, 0.0, 1.0)
    img = screen(img, '#E6EEFF', beam_in * 0.45)
    # 色散扇形光谱（七道柔光波段）
    spec_cols = [
        ('#FF3B7F', -0.22, 72),
        ('#FF7A33', -0.13, 68),
        ('#FFD84D', -0.04, 65),
        ('#3CE8B5',  0.05, 72),
        ('#25B7FF',  0.14, 80),
        ('#5D6BFF',  0.24, 86),
        ('#A955FF',  0.34, 88),
    ]
    for col, slope, sigma in spec_cols:
        # 从左中向右扩散，同时在顶部和左右边缘有高亮度露出
        dist = (ys - (260.0 + (xs - 140.0) * (0.48 + slope)))
        spread = sigma * (0.65 + 0.9 * np.clip(xs / w, 0.0, 1.0))
        band = np.exp(-(dist / spread) ** 2) * (0.55 + 0.45 * np.clip(xs / 400.0, 0.0, 1.0))
        img = screen(img, col, band * 0.62)
        # 下半部对称反射光谱
        dist2 = (ys - (1180.0 - (xs - 120.0) * (0.28 - slope * 0.7)))
        band2 = np.exp(-(dist2 / (spread * 1.1)) ** 2)
        img = screen(img, col, band2 * 0.42)
    # 玻璃棱镜轮廓与微光棱线
    glass = svg_rgba('''
        <polygon points="533,190 920,860 146,860" fill="rgba(255,255,255,0.03)" stroke="rgba(235,244,255,0.22)" stroke-width="2"/>
        <line x1="533" y1="190" x2="533" y2="860" stroke="rgba(255,255,255,0.10)" stroke-width="1.5"/>
        <polygon points="533,1420 820,980 246,980" fill="rgba(255,255,255,0.02)" stroke="rgba(235,244,255,0.14)" stroke-width="1.5"/>
    ''', BG)
    img = over(img, glass)
    bg = save(grain(img, 5.5, 11), 'bg', 'prism')

    # 胶带：黑曜底斜向光谱折射条 + 白色精密刻度
    ticks = ''.join(f'<line x1="{x}" y1="0" x2="{x}" y2="{10 if x % 25 == 0 else 5}" stroke="#ffffff" stroke-opacity="0.35" stroke-width="1"/>' for x in range(0, 100, 5))
    tape = tape_svg('#0E111B', 100, f'''
        <g opacity="0.92">
          <polygon points="10,96 42,0 56,0 24,96" fill="#FF3B7F"/>
          <polygon points="22,96 54,0 66,0 34,96" fill="#FF9233"/>
          <polygon points="32,96 64,0 76,0 44,96" fill="#FFD84D"/>
          <polygon points="42,96 74,0 86,0 54,96" fill="#3CE8B5"/>
          <polygon points="52,96 84,0 96,0 64,96" fill="#25B7FF"/>
          <polygon points="62,96 94,0 106,0 74,96" fill="#7B61FF"/>
        </g>
        <line x1="0" y1="48" x2="100" y2="48" stroke="#ffffff" stroke-opacity="0.22" stroke-width="0.8"/>
        {ticks}
    ''', seed=12)
    save(tape, 'tape', 'prism')

    # 封面：横向黑曜底，中心玻璃棱镜折射出七彩光束
    xs, ys = grid(COVER)
    cov = ramp(xs / COVER[0], [(0.0, '#090B12'), (0.5, '#121624'), (1.0, '#080A11')])
    # 左侧入射白光
    in_b = np.exp(-((ys - (210.0 + (xs - 840.0) * 0.10)) / 18.0) ** 2) * np.clip((880.0 - xs) / 100.0, 0.0, 1.0)
    cov = screen(cov, '#FFFFFF', in_b * 0.72)
    # 右侧展开光谱
    for col, slope, sig in [
        ('#FF3B7F', -0.14, 24), ('#FF8833', -0.09, 24), ('#FFE055', -0.04, 24),
        ('#3CE8B5',  0.01, 26), ('#25B7FF',  0.06, 28), ('#7C63FF',  0.12, 30),
    ]:
        t = np.clip((xs - 950.0) / 900.0, 0.0, 1.0)
        d = ys - (200.0 + (xs - 960.0) * slope)
        s = sig * (0.45 + 1.4 * t)
        b = np.exp(-(d / s) ** 2) * np.clip((xs - 920.0) / 80.0, 0.0, 1.0)
        cov = screen(cov, col, b * 0.78)
    tri = svg_rgba('''
        <polygon points="940,52 1100,348 780,348" fill="rgba(255,255,255,0.06)" stroke="rgba(255,255,255,0.48)" stroke-width="2.5"/>
        <line x1="940" y1="52" x2="940" y2="348" stroke="rgba(255,255,255,0.16)" stroke-width="1.5"/>
    ''', COVER)
    cov = over(cov, tri)
    save(grain(cov, 4.5, 13), 'cover', 'prism')
    return bg


# ---------------------------------------------------------------- 2. 影青开片 (Celadon)
def voronoi_cracks(size, n_pts, seed, width=2.2):
    w, h = size
    rng = np.random.default_rng(seed)
    pts = np.stack([rng.uniform(-0.08 * w, 1.08 * w, n_pts), rng.uniform(-0.08 * h, 1.08 * h, n_pts)], axis=1)
    # 降采样算距离场再升采样，兼顾速度与细腻
    sw, sh = w // 2, h // 2
    ys, xs = np.mgrid[0:sh, 0:sw].astype(np.float32) * 2.0
    d1 = np.full((sh, sw), 1e9, np.float32)
    d2 = np.full((sh, sw), 1e9, np.float32)
    for px, py in pts:
        d = np.hypot(xs - px, ys - py)
        m = d < d1
        d2 = np.where(m, d1, np.minimum(d2, d))
        d1 = np.where(m, d, d1)
    edge = d2 - d1
    im = Image.fromarray(np.clip(edge * 4.0, 0, 255).astype(np.uint8)).resize((w, h), Image.BICUBIC)
    edge_full = np.asarray(im, np.float32) / 4.0
    return np.clip(1.0 - edge_full / width, 0.0, 1.0)


@style
def celadon():
    xs, ys = grid(BG)
    w, h = BG
    n = noise(BG, 240, seed=21, octaves=4)
    t = np.clip(0.55 * (ys / h) + 0.25 * (xs / w) + 0.25 * n, 0.0, 1.0)
    img = ramp(t, [(0.0, '#CBE0D7'), (0.35, '#9DC5B8'), (0.72, '#78A89B'), (1.0, '#5A897F')])
    # 温润釉面柔光
    img = screen(img, '#F5FBF8', glow(BG, 260, 220, 680, 2.2) * 0.45)
    img = screen(img, '#E6F5EE', glow(BG, 880, 1280, 560, 2.0) * 0.30)
    # 主开片（铁线）与次开片（金丝）
    cracks_main = voronoi_cracks(BG, 52, seed=22, width=2.4)
    cracks_sub = voronoi_cracks(BG, 140, seed=23, width=1.4)
    img = blend(img, '#3A5E56', cracks_main * 0.58)
    img = blend(img, '#C5A267', cracks_sub * 0.34)
    # 釉面细气泡颗粒
    bg = save(grain(img, 3.8, 24), 'bg', 'celadon')

    # 胶带：影青釉面 + 冰裂金丝 + 上下细釉边
    tape = tape_svg('#9AC2B5', 200, '''
        <rect x="0" y="0" width="200" height="5" fill="#DCECE5" fill-opacity="0.7"/>
        <rect x="0" y="91" width="200" height="5" fill="#5D877D" fill-opacity="0.45"/>
        <g fill="none" stroke="#3B5D55" stroke-width="1.35" stroke-opacity="0.72" stroke-linecap="round">
          <path d="M0 32 L38 22 L76 48 L128 34 L168 58 L200 32"/>
          <path d="M38 22 L52 0 M76 48 L62 96 M128 34 L144 0 M168 58 L182 96"/>
          <path d="M0 74 L32 66 L76 48 M128 34 L112 76 L168 58 M112 76 L98 96"/>
        </g>
        <g fill="none" stroke="#D8B675" stroke-width="0.9" stroke-opacity="0.75">
          <path d="M18 0 L38 22 M32 66 L14 96 M96 0 L128 34 M76 48 L104 20 L144 0"/>
          <path d="M168 58 L194 18 L200 18"/>
        </g>
    ''', seed=25)
    save(tape, 'tape', 'celadon')

    # 封面：影青釉面配一道优雅金缮线
    xs, ys = grid(COVER)
    nc = noise(COVER, 220, seed=26, octaves=4)
    tc = np.clip(0.6 * (xs / COVER[0]) + 0.25 * (ys / COVER[1]) + 0.2 * nc, 0.0, 1.0)
    cov = ramp(tc, [(0.0, '#CBE0D7'), (0.45, '#99C2B4'), (1.0, '#6B998E')])
    cov = screen(cov, '#F4FBF8', glow(COVER, 560, 120, 620, 2.0) * 0.42)
    cov = blend(cov, '#3B5E56', voronoi_cracks(COVER, 55, seed=27, width=2.0) * 0.48)
    gold_vein = svg_rgba('''
        <g fill="none" stroke-linecap="round" stroke-linejoin="round">
          <path d="M-20 260 Q 260 220 480 250 T 980 185 T 1480 230 T 2020 155" stroke="#8C6A32" stroke-width="8" stroke-opacity="0.45"/>
          <path d="M-20 260 Q 260 220 480 250 T 980 185 T 1480 230 T 2020 155" stroke="#E5C787" stroke-width="4.2"/>
          <path d="M480 250 Q 590 310 710 390 M980 185 Q 1080 110 1190 20 M1480 230 Q 1560 305 1660 390" stroke="#DFBD79" stroke-width="2.2" stroke-opacity="0.88"/>
        </g>
    ''', COVER)
    cov = over(cov, gold_vein)
    save(grain(cov, 3.5, 28), 'cover', 'celadon')
    return bg


# ---------------------------------------------------------------- 3. 泳池焦散 (Caustics)
def caustic_field(size, scale=110.0, seed=31):
    xs, ys = grid(size)
    rng = np.random.default_rng(seed)
    acc = np.zeros_like(xs)
    for _ in range(7):
        ang = rng.uniform(0, math.pi)
        freq = (2.0 * math.pi / scale) * rng.uniform(0.75, 1.35)
        phase = rng.uniform(0, 2.0 * math.pi)
        acc += np.sin(xs * math.cos(ang) * freq + ys * math.sin(ang) * freq + phase)
    # 取接近 0 的脊线做焦散亮网
    net = np.exp(-((np.abs(acc) / 1.15) ** 1.6))
    net2 = np.exp(-((np.abs(acc - 1.4) / 0.85) ** 2.0)) * 0.45
    return np.clip(net + net2, 0.0, 1.0)


@style
def caustics():
    xs, ys = grid(BG)
    w, h = BG
    t = np.clip(0.75 * (ys / h) + 0.25 * (xs / w), 0.0, 1.0)
    img = ramp(t, [(0.0, '#6BD6EA'), (0.35, '#29AEE0'), (0.75, '#127EC2'), (1.0, '#0A5594')])
    c1 = caustic_field(BG, 135.0, seed=31)
    c2 = caustic_field(BG, 78.0, seed=32)
    img = screen(img, '#E6FBFF', c1 * 0.62)
    img = screen(img, '#BDF4FF', c2 * 0.34)
    img = screen(img, '#FFFFFF', glow(BG, 220, 160, 560, 2.0) * 0.30)
    bg = save(grain(img, 3.2, 33), 'bg', 'caustics')

    # 胶带：清澈水波焦散网 + 复古泳道细红线
    tape = tape_svg('#30B2E2', 200, '''
        <g fill="none" stroke="#E8FCFF" stroke-linecap="round">
          <path d="M0 24 Q25 6 50 26 T100 24 T150 28 T200 24" stroke-width="2.2" stroke-opacity="0.82"/>
          <path d="M0 54 Q30 72 60 48 T120 56 T170 46 T200 54" stroke-width="2.6" stroke-opacity="0.9"/>
          <path d="M0 78 Q22 64 46 80 T98 76 T148 82 T200 78" stroke-width="1.8" stroke-opacity="0.72"/>
          <path d="M26 0 Q38 26 24 52 T32 96 M84 0 Q72 30 92 56 T80 96 M146 0 Q160 28 142 56 T152 96" stroke-width="1.5" stroke-opacity="0.68"/>
        </g>
        <line x1="0" y1="11" x2="200" y2="11" stroke="#FF6B5B" stroke-width="2.2" stroke-dasharray="14 8" stroke-opacity="0.88"/>
        <line x1="0" y1="85" x2="200" y2="85" stroke="#FF6B5B" stroke-width="2.2" stroke-dasharray="14 8" stroke-opacity="0.88"/>
    ''', seed=34)
    save(tape, 'tape', 'caustics')

    # 封面：横向地中海日光泳池焦散
    xs, ys = grid(COVER)
    tc = np.clip(0.6 * (ys / COVER[1]) + 0.4 * (xs / COVER[0]), 0.0, 1.0)
    cov = ramp(tc, [(0.0, '#7EE0F2'), (0.45, '#2CB3E3'), (1.0, '#0F6FB3')])
    cov = screen(cov, '#F0FDFF', caustic_field(COVER, 115.0, seed=35) * 0.68)
    cov = screen(cov, '#CCF7FF', caustic_field(COVER, 64.0, seed=36) * 0.35)
    save(grain(cov, 3.0, 37), 'cover', 'caustics')
    return bg


# ---------------------------------------------------------------- 4. 曜石金缮 (Kintsugi)
@style
def obsidian():
    xs, ys = grid(BG)
    n = noise(BG, 180, seed=41, octaves=5)
    img = ramp(np.clip(0.35 * (ys / BG[1]) + 0.65 * n, 0.0, 1.0), [(0.0, '#1B1917'), (0.5, '#121110'), (1.0, '#0A0909')])
    # 柔和陶面微光
    img = screen(img, '#42362A', glow(BG, 820, 260, 620, 2.2) * 0.25)
    # 主金缮脉络（边缘留白处皆有穿过）
    veins = svg_rgba('''
        <defs>
          <linearGradient id="g1" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#FCE4A6"/>
            <stop offset="38%" stop-color="#D9A84E"/>
            <stop offset="70%" stop-color="#FFF0C4"/>
            <stop offset="100%" stop-color="#A6782B"/>
          </linearGradient>
        </defs>
        <g fill="none" stroke-linecap="round" stroke-linejoin="round">
          <path d="M-20 155 Q 190 120 340 210 T 680 140 T 1090 220" stroke="#704D18" stroke-width="18" stroke-opacity="0.35"/>
          <path d="M-20 155 Q 190 120 340 210 T 680 140 T 1090 220" stroke="url(#g1)" stroke-width="8.5"/>
          <path d="M120 165 Q 60 440 130 760 T 70 1320 T 180 1620" stroke="#704D18" stroke-width="13" stroke-opacity="0.3"/>
          <path d="M120 165 Q 60 440 130 760 T 70 1320 T 180 1620" stroke="url(#g1)" stroke-width="5.5"/>
          <path d="M950 185 Q 1010 520 930 880 T 990 1420" stroke="url(#g1)" stroke-width="5.0"/>
          <path d="M-20 1390 Q 320 1310 560 1420 T 1090 1340" stroke="url(#g1)" stroke-width="7.0"/>
          <path d="M340 210 Q 430 330 510 380 M680 140 Q 750 60 820 -10 M930 880 Q 820 960 760 1050 M560 1420 Q 510 1510 470 1620" stroke="url(#g1)" stroke-width="2.6" stroke-opacity="0.9"/>
        </g>
    ''', BG)
    img = over(img, veins)
    bg = save(grain(img, 5.2, 42), 'bg', 'obsidian')

    # 胶带：哑光黑漆底 + 连绵金缮脉络 + 细金砂点
    rng = np.random.default_rng(43)
    specks = ''.join(f'<circle cx="{rng.uniform(4, 396):.1f}" cy="{rng.uniform(8, 88):.1f}" r="{rng.uniform(0.7, 1.8):.1f}" fill="#EBC878" fill-opacity="{rng.uniform(0.35, 0.85):.2f}"/>' for _ in range(36))
    tape = tape_svg('#151311', 400, f'''
        <defs>
          <linearGradient id="tg" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#D4A248"/><stop offset="35%" stop-color="#FCE6AD"/>
            <stop offset="70%" stop-color="#C6923B"/><stop offset="100%" stop-color="#D4A248"/>
          </linearGradient>
        </defs>
        <path d="M0 52 Q 55 24 110 48 T 220 42 T 320 62 T 400 52" fill="none" stroke="url(#tg)" stroke-width="4.2"/>
        <path d="M110 48 Q 135 76 165 96 M220 42 Q 240 16 262 0 M320 62 Q 345 80 368 96 M48 36 Q 68 12 86 0" fill="none" stroke="#E8C272" stroke-width="1.8" stroke-opacity="0.88"/>
        {specks}
    ''', seed=44)
    save(tape, 'tape', 'obsidian')

    # 封面：横幅曜石金缮
    nc = noise(COVER, 180, seed=45, octaves=5)
    cov = ramp(np.clip(0.35 * (grid(COVER)[0] / COVER[0]) + 0.65 * nc, 0.0, 1.0), [(0.0, '#1C1917'), (0.5, '#12100F'), (1.0, '#0B0A09')])
    cov_v = svg_rgba('''
        <defs>
          <linearGradient id="cg" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#C28E36"/><stop offset="30%" stop-color="#FDE8B2"/>
            <stop offset="65%" stop-color="#D8A64A"/><stop offset="100%" stop-color="#F7DC9A"/>
          </linearGradient>
        </defs>
        <g fill="none" stroke-linecap="round">
          <path d="M-20 220 Q 320 110 640 230 T 1320 175 T 2020 215" stroke="#7A5318" stroke-width="16" stroke-opacity="0.35"/>
          <path d="M-20 220 Q 320 110 640 230 T 1320 175 T 2020 215" stroke="url(#cg)" stroke-width="7"/>
          <path d="M360 168 Q 460 72 560 -10 M640 230 Q 760 315 890 410 M1120 198 Q 1200 95 1310 -10 M1580 188 Q 1690 295 1780 410" stroke="url(#cg)" stroke-width="3.2"/>
        </g>
    ''', COVER)
    cov = over(cov, cov_v)
    save(grain(cov, 4.8, 46), 'cover', 'obsidian')
    return bg


# ---------------------------------------------------------------- 5. 二向色玻璃 (Dichroic)
@style
def dichroic():
    xs, ys = grid(BG)
    img = ramp(ys / BG[1], [(0.0, '#0D1028'), (0.5, '#131738'), (1.0, '#090C1E')])
    # 多块半透明二向色玻璃叠加
    panes = svg_rgba('''
        <defs>
          <linearGradient id="d1" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#FF3EA5" stop-opacity="0.72"/>
            <stop offset="50%" stop-color="#6B5BFF" stop-opacity="0.45"/>
            <stop offset="100%" stop-color="#00E5FF" stop-opacity="0.78"/>
          </linearGradient>
          <linearGradient id="d2" x1="100%" y1="0%" x2="0%" y2="100%">
            <stop offset="0%" stop-color="#FFB830" stop-opacity="0.72"/>
            <stop offset="50%" stop-color="#FF4785" stop-opacity="0.48"/>
            <stop offset="100%" stop-color="#38F2C0" stop-opacity="0.65"/>
          </linearGradient>
          <linearGradient id="d3" x1="0%" y1="100%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#1E6BFF" stop-opacity="0.78"/>
            <stop offset="50%" stop-color="#00F0D0" stop-opacity="0.52"/>
            <stop offset="100%" stop-color="#FFE066" stop-opacity="0.75"/>
          </linearGradient>
        </defs>
        <polygon points="-40,120 760,-40 940,680 120,820" fill="url(#d1)" stroke="rgba(255,255,255,0.42)" stroke-width="2"/>
        <polygon points="240,260 1120,100 1120,1080 340,1180" fill="url(#d2)" stroke="rgba(255,255,255,0.38)" stroke-width="2"/>
        <polygon points="-60,860 840,720 1020,1640 -60,1640" fill="url(#d3)" stroke="rgba(255,255,255,0.42)" stroke-width="2"/>
        <line x1="120" y1="820" x2="940" y2="680" stroke="rgba(255,255,255,0.65)" stroke-width="2.5"/>
        <line x1="240" y1="260" x2="1120" y2="100" stroke="rgba(255,255,255,0.55)" stroke-width="2"/>
    ''', BG)
    img = over(img, panes)
    img = screen(img, '#7CEEFF', glow(BG, 860, 180, 480, 2.2) * 0.35)
    img = screen(img, '#FF75C3', glow(BG, 180, 1340, 520, 2.2) * 0.32)
    bg = save(grain(img, 4.2, 51), 'bg', 'dichroic')

    # 胶带：深蓝底二向色菱格折射片
    tape = tape_svg('#10142D', 160, '''
        <defs>
          <linearGradient id="dt1" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#FF3EA5"/><stop offset="50%" stop-color="#7B61FF"/><stop offset="100%" stop-color="#00E5FF"/>
          </linearGradient>
          <linearGradient id="dt2" x1="0%" y1="100%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#FFB830"/><stop offset="50%" stop-color="#FF4785"/><stop offset="100%" stop-color="#38F2C0"/>
          </linearGradient>
        </defs>
        <polygon points="40,6 76,48 40,90 4,48" fill="url(#dt1)" fill-opacity="0.88" stroke="#FFFFFF" stroke-opacity="0.55" stroke-width="1.2"/>
        <polygon points="120,6 156,48 120,90 84,48" fill="url(#dt2)" fill-opacity="0.85" stroke="#FFFFFF" stroke-opacity="0.55" stroke-width="1.2"/>
        <polygon points="80,22 104,48 80,74 56,48" fill="#FFFFFF" fill-opacity="0.22" stroke="#FFFFFF" stroke-opacity="0.7" stroke-width="1"/>
        <polygon points="0,22 24,48 0,74 -24,48" fill="#FFFFFF" fill-opacity="0.22" stroke="#FFFFFF" stroke-opacity="0.7" stroke-width="1"/>
    ''', seed=52)
    save(tape, 'tape', 'dichroic')

    # 封面：横向交叠二向色玻璃板
    cov = ramp(grid(COVER)[0] / COVER[0], [(0.0, '#0C0F26'), (0.5, '#151A3E'), (1.0, '#0A0D20')])
    cov_p = svg_rgba('''
        <defs>
          <linearGradient id="cd1" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#FF3EA5" stop-opacity="0.78"/><stop offset="100%" stop-color="#00E5FF" stop-opacity="0.78"/>
          </linearGradient>
          <linearGradient id="cd2" x1="0%" y1="100%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#FFB830" stop-opacity="0.78"/><stop offset="100%" stop-color="#38F2C0" stop-opacity="0.72"/>
          </linearGradient>
          <linearGradient id="cd3" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#5B4BFF" stop-opacity="0.72"/><stop offset="100%" stop-color="#FF6BA9" stop-opacity="0.72"/>
          </linearGradient>
        </defs>
        <polygon points="140,-20 760,-20 620,420 0,420" fill="url(#cd1)" stroke="rgba(255,255,255,0.5)" stroke-width="2"/>
        <polygon points="580,-20 1320,-20 1440,420 700,420" fill="url(#cd2)" stroke="rgba(255,255,255,0.5)" stroke-width="2"/>
        <polygon points="1220,-20 1920,-20 1760,420 1060,420" fill="url(#cd3)" stroke="rgba(255,255,255,0.5)" stroke-width="2"/>
    ''', COVER)
    cov = over(cov, cov_p)
    save(grain(cov, 4.0, 53), 'cover', 'dichroic')
    return bg


# ---------------------------------------------------------------- 6. 陶土拱廊 (Terracotta)
@style
def terracotta():
    # Luis Barragán 风格建筑光影与赤陶灰泥肌理
    n = noise(BG, 90, seed=61, octaves=5)
    img = canvas(BG, '#D97352')
    arch = svg_rgba('''
        <rect x="0" y="0" width="1066" height="1600" fill="#DE7654"/>
        <!-- 顶部晴空窄带与阶梯墙 -->
        <rect x="0" y="0" width="1066" height="185" fill="#2A5EA8"/>
        <polygon points="0,120 420,120 420,1600 0,1600" fill="#E89476"/>
        <polygon points="420,185 1066,185 1066,1600 420,1600" fill="#C95A3B"/>
        <!-- 经典半圆拱门与深邃斜影 -->
        <path d="M120 1600 L120 560 A320 320 0 0 1 760 560 L760 1600 Z" fill="#F2C2B0"/>
        <path d="M120 560 A320 320 0 0 1 760 560 L760 660 A260 260 0 0 0 240 660 L240 1600 L120 1600 Z" fill="#8E3422" fill-opacity="0.72"/>
        <!-- 右侧太阳斜影 -->
        <polygon points="760,240 1066,120 1066,1600 760,1460" fill="#A63F28" fill-opacity="0.58"/>
        <!-- 底部暖粉几何台 -->
        <polygon points="0,1240 1066,1360 1066,1600 0,1600" fill="#F0B5A4"/>
        <polygon points="0,1360 620,1460 620,1600 0,1600" fill="#B94B31" fill-opacity="0.48"/>
    ''', BG)
    img = over(img, arch)
    # 灰泥墙面质感
    img = img + (n[..., None] - 0.5) * 22.0
    bg = save(grain(img, 5.0, 62), 'bg', 'terracotta')

    # 胶带：赤陶底连续建筑拱门与蓝色晴空窗口
    tape = tape_svg('#D46646', 100, '''
        <rect x="0" y="0" width="100" height="14" fill="#2B5EA8"/>
        <path d="M14 96 L14 48 A36 36 0 0 1 86 48 L86 96 Z" fill="#F4C7B6"/>
        <path d="M14 48 A36 36 0 0 1 86 48 L86 58 A28 28 0 0 0 28 58 L28 96 L14 96 Z" fill="#8C3422" fill-opacity="0.72"/>
        <rect x="0" y="88" width="100" height="8" fill="#A63E26"/>
    ''', seed=63, grain_amt=3.0)
    save(tape, 'tape', 'terracotta')

    # 封面：横向赤陶拱廊列阵
    nc = noise(COVER, 80, seed=64, octaves=5)
    arches = ''.join(f'''
        <path d="M{x} 400 L{x} 165 A110 110 0 0 1 {x + 220} 165 L{x + 220} 400 Z" fill="#F3C5B4"/>
        <path d="M{x} 165 A110 110 0 0 1 {x + 220} 165 L{x + 220} 200 A92 92 0 0 0 {x + 42} 200 L{x + 42} 400 L{x} 400 Z" fill="#8D3422" fill-opacity="0.72"/>
    ''' for x in range(80, 1960, 290))
    cov = svg_rgba(f'''
        <rect width="2000" height="400" fill="#D96E4D"/>
        <rect width="2000" height="62" fill="#295DA6"/>
        {arches}
        <polygon points="0,335 2000,368 2000,400 0,400" fill="#B84B30" fill-opacity="0.52"/>
    ''', COVER)[..., :3]
    cov = cov + (nc[..., None] - 0.5) * 18.0
    save(grain(cov, 4.5, 65), 'cover', 'terracotta')
    return bg


# ---------------------------------------------------------------- 7. 极光帷幕 (Aurora)
@style
def aurora():
    xs, ys = grid(BG)
    w, h = BG
    img = ramp(ys / h, [(0.0, '#07111C'), (0.55, '#0B1D2C'), (1.0, '#060D17')])
    # 丝绸状垂落极光帷幕（三层相位波 + 垂直光柱衰减）
    x_norm = xs / w
    y_norm = ys / h
    for base_y, amp, freq, phase, col, thickness, strength in [
        (0.18, 0.08, 2.2, 0.4, '#24F0A8', 0.20, 0.72),
        (0.32, 0.11, 1.7, 1.8, '#17D4E8', 0.24, 0.58),
        (0.24, 0.09, 2.8, 3.2, '#9A5CFF', 0.18, 0.42),
        (0.76, 0.10, 2.0, 2.5, '#2AF5B4', 0.22, 0.52),
        (0.84, 0.08, 2.5, 0.9, '#6C63FF', 0.20, 0.40),
    ]:
        ridge = base_y + amp * np.sin(2.0 * math.pi * x_norm * freq + phase) + 0.035 * np.sin(2.0 * math.pi * x_norm * freq * 2.3 + phase * 1.7)
        dy = y_norm - ridge
        # 下缘锐利、向上拖出柔和丝绸光幕
        curtain = np.where(dy < 0.0, np.exp(-(np.maximum(-dy, 0.0) / thickness) ** 1.4), np.exp(-((dy) / (thickness * 0.22)) ** 2.0))
        ribbons = 0.62 + 0.38 * np.sin(xs * 0.055 + phase * 4.0 + np.sin(xs * 0.014) * 3.0)
        img = screen(img, col, curtain * ribbons * strength)
    # 极细星尘
    rng = np.random.default_rng(71)
    stars = Image.new('L', BG, 0)
    d = ImageDraw.Draw(stars)
    for _ in range(260):
        sx, sy = int(rng.uniform(0, w)), int(rng.uniform(0, h))
        r = rng.choice([1, 1, 1, 2])
        val = int(rng.uniform(110, 255))
        d.ellipse((sx - r, sy - r, sx + r, sy + r), fill=val)
    img = screen(img, '#E8FAFF', np.asarray(stars, np.float32) / 255.0 * 0.65)
    bg = save(grain(img, 3.8, 72), 'bg', 'aurora')

    # 胶带：深极夜底垂直极光丝带频谱 + 星点
    xs_t, ys_t = grid(TAPE)
    t_img = canvas(TAPE, '#081420')
    xn = xs_t / TAPE[0]
    yn = ys_t / TAPE[1]
    for freq, ph, col, strg in [(2, 0.0, '#24F0A8', 0.85), (4, 1.3, '#17D4E8', 0.65), (3, 2.4, '#9B5CFF', 0.55)]:
        wave = 0.58 + 0.20 * np.sin(2.0 * math.pi * xn * freq + ph)
        dy = yn - wave
        c = np.where(dy < 0, np.exp(-(np.maximum(-dy, 0.0) / 0.42) ** 1.5), np.exp(-((dy) / 0.14) ** 2.0))
        rib = 0.65 + 0.35 * np.sin(2.0 * math.pi * xn * 24.0 + ph)
        t_img = screen(t_img, col, c * rib * strg)
    save(grain(t_img, 2.5, 73), 'tape', 'aurora')

    # 封面：横向极夜极光天际线
    xs, ys = grid(COVER)
    cov = ramp(ys / COVER[1], [(0.0, '#07101B'), (0.65, '#0C1E2E'), (1.0, '#08121D')])
    xn, yn = xs / COVER[0], ys / COVER[1]
    for by, amp, freq, ph, col, th, st in [
        (0.68, 0.14, 1.5, 0.5, '#24F0A8', 0.42, 0.82),
        (0.74, 0.12, 2.2, 1.9, '#19D4E8', 0.36, 0.65),
        (0.58, 0.15, 1.8, 3.4, '#9B5CFF', 0.38, 0.52),
    ]:
        ridge = by + amp * np.sin(2.0 * math.pi * xn * freq + ph)
        dy = yn - ridge
        curtain = np.where(dy < 0.0, np.exp(-(np.maximum(-dy, 0.0) / th) ** 1.4), np.exp(-((dy) / (th * 0.22)) ** 2.0))
        rib = 0.62 + 0.38 * np.sin(xs * 0.04 + ph * 3.0)
        cov = screen(cov, col, curtain * rib * st)
    save(grain(cov, 3.5, 74), 'cover', 'aurora')
    return bg


# ---------------------------------------------------------------- 8. 朱漆螺钿 (Lacquer)
@style
def vermilion():
    xs, ys = grid(BG)
    n = noise(BG, 200, seed=81, octaves=4)
    # 温润朱砂大漆：中心略明，边缘沉厚如漆器包浆
    dist = np.hypot((xs - 533.0) / 720.0, (ys - 760.0) / 1000.0)
    t = np.clip(0.55 * dist + 0.25 * n, 0.0, 1.0)
    img = ramp(t, [(0.0, '#C83728'), (0.55, '#A9241B'), (1.0, '#721410')])
    # 漆面柔润反光
    img = screen(img, '#FF9E82', glow(BG, 260, 180, 560, 2.4) * 0.26)
    # 现代极简描金弧线 + 螺钿贝母菱片
    deco = svg_rgba('''
        <defs>
          <linearGradient id="m1" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#DDFBF8"/><stop offset="45%" stop-color="#F6D8F8"/><stop offset="100%" stop-color="#C8F2DF"/>
          </linearGradient>
        </defs>
        <g fill="none" stroke="#EBC97E" stroke-opacity="0.72">
          <circle cx="533" cy="240" r="320" stroke-width="1.8"/>
          <circle cx="533" cy="240" r="356" stroke-width="1.0" stroke-dasharray="10 8"/>
          <circle cx="90" cy="820" r="240" stroke-width="1.6"/>
          <circle cx="976" cy="1280" r="280" stroke-width="1.8"/>
          <path d="M-20 1440 Q 260 1320 533 1420 T 1090 1360" stroke-width="2.2"/>
          <path d="M-20 1468 Q 260 1348 533 1448 T 1090 1388" stroke-width="1.1" stroke-opacity="0.55"/>
        </g>
        <!-- 螺钿贝母几何星点 -->
        <g fill="url(#m1)" fill-opacity="0.88">
          <polygon points="180,120 198,144 180,168 162,144"/>
          <polygon points="880,108 902,138 880,168 858,138"/>
          <polygon points="74,560 90,582 74,604 58,582"/>
          <polygon points="992,690 1010,714 992,738 974,714"/>
          <polygon points="82,1180 100,1204 82,1228 64,1204"/>
          <polygon points="950,1480 970,1506 950,1532 930,1506"/>
          <polygon points="310,1490 326,1512 310,1534 294,1512"/>
        </g>
    ''', BG)
    img = over(img, deco)
    bg = save(grain(img, 3.8, 82), 'bg', 'vermilion')

    # 胶带：朱红漆底 + 描金菱格与螺钿贝母菱片
    tape = tape_svg('#A9241B', 100, '''
        <defs>
          <linearGradient id="mt" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#D8FAF6"/><stop offset="50%" stop-color="#F7D8FA"/><stop offset="100%" stop-color="#C9F3DD"/>
          </linearGradient>
        </defs>
        <line x1="0" y1="8" x2="100" y2="8" stroke="#EBC97E" stroke-width="1.4"/>
        <line x1="0" y1="88" x2="100" y2="88" stroke="#EBC97E" stroke-width="1.4"/>
        <path d="M0 48 L25 22 L50 48 L75 74 L100 48 M0 48 L25 74 L50 48 L75 22 L100 48" fill="none" stroke="#EBC97E" stroke-width="1.3" stroke-opacity="0.82"/>
        <polygon points="50,30 62,48 50,66 38,48" fill="url(#mt)" fill-opacity="0.92"/>
        <circle cx="0" cy="48" r="4.5" fill="#F2D48E"/>
        <circle cx="100" cy="48" r="4.5" fill="#F2D48E"/>
    ''', seed=83)
    save(tape, 'tape', 'vermilion')

    # 封面：横向朱漆描金流云与螺钿
    xs, ys = grid(COVER)
    nc = noise(COVER, 180, seed=84, octaves=4)
    tc = np.clip(0.5 * np.abs(xs / COVER[0] - 0.5) * 1.4 + 0.25 * nc, 0.0, 1.0)
    cov = ramp(tc, [(0.0, '#C53527'), (0.6, '#A42118'), (1.0, '#721410')])
    cov_d = svg_rgba('''
        <defs>
          <linearGradient id="mc" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#DDFBF8"/><stop offset="50%" stop-color="#F6D8F8"/><stop offset="100%" stop-color="#C8F2DF"/>
          </linearGradient>
        </defs>
        <g fill="none" stroke="#ECCB80" stroke-opacity="0.78">
          <path d="M-20 280 Q 340 120 680 240 T 1360 190 T 2020 250" stroke-width="2.6"/>
          <path d="M-20 310 Q 340 150 680 270 T 1360 220 T 2020 280" stroke-width="1.2" stroke-opacity="0.55"/>
          <circle cx="1000" cy="200" r="140" stroke-width="1.8"/>
          <circle cx="1000" cy="200" r="168" stroke-width="1.0" stroke-dasharray="10 8"/>
        </g>
        <g fill="url(#mc)" fill-opacity="0.9">
          <polygon points="1000,148 1032,200 1000,252 968,200"/>
          <polygon points="460,156 476,180 460,204 444,180"/>
          <polygon points="1540,176 1556,200 1540,224 1524,200"/>
        </g>
    ''', COVER)
    cov = over(cov, cov_d)
    save(grain(cov, 3.6, 85), 'cover', 'vermilion')
    return bg


# ---------------------------------------------------------------- 9. 国际克莱因 (Cobalt)
@style
def cobalt():
    xs, ys = grid(BG)
    # 极纯群青厚涂颜料肌理（细粗双层噪声模拟亚光颜料颗粒）
    n1 = noise(BG, 140, seed=91, octaves=4)
    n2 = noise(BG, 18, seed=92, octaves=3)
    img = ramp(np.clip(0.55 * n1 + 0.45 * n2, 0.0, 1.0), [(0.0, '#00229E'), (0.5, '#002FA7'), (1.0, '#1044C8')])
    # 极简美术馆建筑基准线与十字准星
    marks = svg_rgba('''
        <g stroke="#FFFFFF" stroke-opacity="0.45" fill="none">
          <line x1="68" y1="0" x2="68" y2="1600" stroke-width="1.2"/>
          <line x1="998" y1="0" x2="998" y2="1600" stroke-width="1.2"/>
          <line x1="0" y1="148" x2="1066" y2="148" stroke-width="1.2"/>
          <line x1="0" y1="1452" x2="1066" y2="1452" stroke-width="1.2"/>
          <circle cx="68" cy="148" r="14" stroke-width="1.5"/>
          <circle cx="998" cy="148" r="14" stroke-width="1.5"/>
          <circle cx="68" cy="1452" r="14" stroke-width="1.5"/>
          <circle cx="998" cy="1452" r="14" stroke-width="1.5"/>
          <circle cx="533" cy="800" r="420" stroke-width="1.2" stroke-opacity="0.28"/>
          <line x1="533" y1="54" x2="533" y2="114" stroke-width="1.5"/>
          <line x1="503" y1="84" x2="563" y2="84" stroke-width="1.5"/>
        </g>
    ''', BG)
    img = over(img, marks)
    bg = save(grain(img, 7.2, 93), 'bg', 'cobalt')

    # 胶带：纯群青底白色建筑十字准星与精密标尺
    ticks = ''.join(f'<line x1="{x}" y1="0" x2="{x}" y2="{14 if x % 25 == 0 else 7}" stroke="#FFFFFF" stroke-opacity="0.72" stroke-width="1.2"/>' for x in range(0, 100, 5))
    tape = tape_svg('#002FA7', 100, f'''
        {ticks}
        <line x1="0" y1="48" x2="100" y2="48" stroke="#FFFFFF" stroke-opacity="0.35" stroke-width="1"/>
        <circle cx="50" cy="48" r="16" fill="none" stroke="#FFFFFF" stroke-width="1.6" stroke-opacity="0.88"/>
        <line x1="50" y1="24" x2="50" y2="72" stroke="#FFFFFF" stroke-width="1.4" stroke-opacity="0.88"/>
        <line x1="26" y1="48" x2="74" y2="48" stroke="#FFFFFF" stroke-width="1.4" stroke-opacity="0.88"/>
        <circle cx="0" cy="48" r="3.5" fill="#FFFFFF" fill-opacity="0.8"/>
        <circle cx="100" cy="48" r="3.5" fill="#FFFFFF" fill-opacity="0.8"/>
    ''', seed=94, grain_amt=4.5)
    save(tape, 'tape', 'cobalt')

    # 封面：横向群青颜料肌理 + 白色黄金比例几何构图线
    n1c = noise(COVER, 140, seed=95, octaves=4)
    n2c = noise(COVER, 18, seed=96, octaves=3)
    cov = ramp(np.clip(0.55 * n1c + 0.45 * n2c, 0.0, 1.0), [(0.0, '#00229E'), (0.5, '#002FA7'), (1.0, '#1044C8')])
    cov_m = svg_rgba('''
        <g stroke="#FFFFFF" stroke-opacity="0.52" fill="none">
          <line x1="0" y1="200" x2="2000" y2="200" stroke-width="1.2"/>
          <line x1="1236" y1="0" x2="1236" y2="400" stroke-width="1.2"/>
          <circle cx="1236" cy="200" r="145" stroke-width="1.8"/>
          <circle cx="1236" cy="200" r="16" stroke-width="1.8"/>
          <line x1="1200" y1="200" x2="1272" y2="200" stroke-width="1.6"/>
          <line x1="1236" y1="164" x2="1236" y2="236" stroke-width="1.6"/>
          <rect x="618" y="55" width="618" height="290" stroke-width="1.2" stroke-opacity="0.36"/>
        </g>
    ''', COVER)
    cov = over(cov, cov_m)
    save(grain(cov, 6.8, 97), 'cover', 'cobalt')
    return bg


# ---------------------------------------------------------------- 10. 枯山水 (Karesansui)
@style
def matcha():
    xs, ys = grid(BG)
    # 枯山水白砂耙纹：水平微弧沙沟 + 两处置石同心圆涟漪
    r1 = np.hypot(xs - 210.0, ys - 290.0)
    r2 = np.hypot(xs - 880.0, ys - 1290.0)
    # 沙沟高度场（0-1）
    base_wave = np.sin((ys + 22.0 * np.sin(xs / 260.0)) * 2.0 * math.pi / 26.0)
    ring1 = np.sin(r1 * 2.0 * math.pi / 26.0)
    ring2 = np.sin(r2 * 2.0 * math.pi / 26.0)
    w1 = np.clip(1.0 - (r1 - 120.0) / 220.0, 0.0, 1.0)
    w2 = np.clip(1.0 - (r2 - 140.0) / 240.0, 0.0, 1.0)
    field = base_wave * (1.0 - w1) * (1.0 - w2) + ring1 * w1 + ring2 * w2
    # 侧光浮雕明暗
    gy, gx = np.gradient(field)
    shade = np.clip(0.5 + 0.35 * gy + 0.15 * gx, 0.0, 1.0)
    img = ramp(shade, [(0.0, '#A8B89E'), (0.45, '#CED9C5'), (1.0, '#E9EFE2')])
    # 置石与青苔点缀
    stones = svg_rgba('''
        <ellipse cx="218" cy="300" rx="88" ry="62" transform="rotate(-14 218 300)" fill="#3F4D3F" fill-opacity="0.22"/>
        <ellipse cx="210" cy="290" rx="82" ry="56" transform="rotate(-14 210 290)" fill="#4D5C4C"/>
        <ellipse cx="195" cy="278" rx="48" ry="30" transform="rotate(-14 195 278)" fill="#6E8266" fill-opacity="0.55"/>
        <ellipse cx="890" cy="1302" rx="108" ry="74" transform="rotate(18 890 1302)" fill="#3A473A" fill-opacity="0.22"/>
        <ellipse cx="880" cy="1290" rx="102" ry="68" transform="rotate(18 880 1290)" fill="#465446"/>
        <ellipse cx="860" cy="1276" rx="56" ry="34" transform="rotate(18 860 1276)" fill="#6D8465" fill-opacity="0.52"/>
    ''', BG)
    img = over(img, stones)
    bg = save(grain(img, 5.2, 101), 'bg', 'matcha')

    # 胶带：白砂耙纹浮雕 + 细苔藓绿边线
    tape = tape_svg('#D4DEC9', 160, '''
        <g fill="none" stroke-linecap="round">
          <path d="M0 16 Q40 8 80 16 T160 16 M0 32 Q40 24 80 32 T160 32 M0 48 Q40 40 80 48 T160 48 M0 64 Q40 56 80 64 T160 64 M0 80 Q40 72 80 80 T160 80" stroke="#8FA184" stroke-width="2.4"/>
          <path d="M0 18 Q40 10 80 18 T160 18 M0 34 Q40 26 80 34 T160 34 M0 50 Q40 42 80 50 T160 50 M0 66 Q40 58 80 66 T160 66 M0 82 Q40 74 80 82 T160 82" stroke="#F0F5EA" stroke-width="1.4"/>
        </g>
        <ellipse cx="80" cy="48" rx="18" ry="12" transform="rotate(-12 80 48)" fill="#4E5F4D"/>
        <ellipse cx="76" cy="45" rx="9" ry="5" transform="rotate(-12 76 45)" fill="#7A9371"/>
    ''', seed=102, grain_amt=3.2)
    save(tape, 'tape', 'matcha')

    # 封面：横向枯山水砂纹与双置石
    xs, ys = grid(COVER)
    r1 = np.hypot(xs - 580.0, ys - 210.0)
    r2 = np.hypot(xs - 1420.0, ys - 185.0)
    base_w = np.sin(ys * 2.0 * math.pi / 24.0)
    w1 = np.clip(1.0 - (r1 - 80.0) / 190.0, 0.0, 1.0)
    w2 = np.clip(1.0 - (r2 - 70.0) / 180.0, 0.0, 1.0)
    f = base_w * (1.0 - w1) * (1.0 - w2) + np.sin(r1 * 2.0 * math.pi / 24.0) * w1 + np.sin(r2 * 2.0 * math.pi / 24.0) * w2
    gy, gx = np.gradient(f)
    cov = ramp(np.clip(0.5 + 0.35 * gy + 0.12 * gx, 0.0, 1.0), [(0.0, '#A8B89E'), (0.45, '#CED9C5'), (1.0, '#EBF1E4')])
    st = svg_rgba('''
        <ellipse cx="586" cy="218" rx="68" ry="44" transform="rotate(-10 586 218)" fill="#3F4D3F" fill-opacity="0.22"/>
        <ellipse cx="580" cy="210" rx="64" ry="40" transform="rotate(-10 580 210)" fill="#4D5C4C"/>
        <ellipse cx="566" cy="200" rx="34" ry="18" transform="rotate(-10 566 200)" fill="#708768" fill-opacity="0.55"/>
        <ellipse cx="1426" cy="193" rx="54" ry="36" transform="rotate(14 1426 193)" fill="#3A473A" fill-opacity="0.22"/>
        <ellipse cx="1420" cy="185" rx="50" ry="32" transform="rotate(14 1420 185)" fill="#465446"/>
    ''', COVER)
    cov = over(cov, st)
    save(grain(cov, 4.8, 103), 'cover', 'matcha')
    return bg


# ---------------------------------------------------------------- 11. 琥珀蜜蜡 (Amber)
@style
def amber():
    xs, ys = grid(BG)
    n = noise(BG, 220, seed=111, octaves=4)
    d = np.hypot((xs - 533.0) / 640.0, (ys - 740.0) / 960.0)
    t = np.clip(0.75 * d + 0.25 * n, 0.0, 1.0)
    img = ramp(t, [(0.0, '#F7B733'), (0.35, '#D46F15'), (0.72, '#7B2D0B'), (1.0, '#2B0E05')])
    # 内部透光柔晕（顶部与边缘也能看到蜜金暖光）
    img = screen(img, '#FFE08A', glow(BG, 320, 150, 480, 2.2) * 0.52)
    img = screen(img, '#FFB84D', glow(BG, 880, 1340, 520, 2.2) * 0.45)
    # 悬浮微小气泡
    rng = np.random.default_rng(112)
    bubbles = []
    for _ in range(95):
        bx, by = rng.uniform(20, BG[0] - 20), rng.uniform(20, BG[1] - 20)
        br = rng.uniform(2.5, 16.0) ** 1.05
        op = rng.uniform(0.18, 0.55)
        bubbles.append(f'<circle cx="{bx:.1f}" cy="{by:.1f}" r="{br:.1f}" fill="#FFE8A3" fill-opacity="{op * 0.55:.2f}" stroke="#FFF4CE" stroke-opacity="{op:.2f}" stroke-width="1.2"/>')
        bubbles.append(f'<circle cx="{bx - br * 0.3:.1f}" cy="{by - br * 0.3:.1f}" r="{max(1.0, br * 0.24):.1f}" fill="#FFFFFF" fill-opacity="{op * 0.9:.2f}"/>')
    img = over(img, svg_rgba(''.join(bubbles), BG))
    bg = save(grain(img, 3.6, 113), 'bg', 'amber')

    # 胶带：琥珀蜜金渐变 + 悬浮发光微泡
    rng = np.random.default_rng(114)
    tb = []
    for _ in range(28):
        bx, by, br = rng.uniform(10, 190), rng.uniform(10, 86), rng.uniform(2.2, 9.5)
        tb.append(f'<circle cx="{bx:.1f}" cy="{by:.1f}" r="{br:.1f}" fill="#FFE699" fill-opacity="0.34" stroke="#FFF4CC" stroke-opacity="0.65" stroke-width="0.9"/>')
        tb.append(f'<circle cx="{bx - br * 0.3:.1f}" cy="{by - br * 0.3:.1f}" r="{max(0.8, br * 0.25):.1f}" fill="#FFFFFF" fill-opacity="0.8"/>')
    tape = tape_svg('#9A3E0E', 200, ''.join(tb) + '''
        <rect x="0" y="0" width="200" height="96" fill="url(#ag)" fill-opacity="0.55"/>
    ''', defs='''
        <linearGradient id="ag" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stop-color="#6B2408"/><stop offset="50%" stop-color="#F2A92A"/><stop offset="100%" stop-color="#6B2408"/>
        </linearGradient>
    ''', seed=115)
    save(tape, 'tape', 'amber')

    # 封面：横向透光老琥珀核心
    xs, ys = grid(COVER)
    nc = noise(COVER, 180, seed=116, octaves=4)
    dc = np.hypot((xs - 1000.0) / 1100.0, (ys - 200.0) / 280.0)
    cov = ramp(np.clip(0.75 * dc + 0.25 * nc, 0.0, 1.0), [(0.0, '#F8BC38'), (0.38, '#D06B14'), (0.75, '#762B0A'), (1.0, '#2B0E05')])
    rng = np.random.default_rng(117)
    cb = ''.join(f'<circle cx="{rng.uniform(40, 1960):.1f}" cy="{rng.uniform(24, 376):.1f}" r="{rng.uniform(2.5, 11.0):.1f}" fill="#FFEAA6" fill-opacity="0.28" stroke="#FFF4CE" stroke-opacity="0.58" stroke-width="1"/>' for _ in range(55))
    cov = over(cov, svg_rgba(cb, COVER))
    save(grain(cov, 3.4, 118), 'cover', 'amber')
    return bg


# ---------------------------------------------------------------- 12. 银盐胶片 (Silver Halide)
@style
def silver():
    xs, ys = grid(BG)
    w, h = BG
    # 暗房银盐黑白光影：斜向百叶窗光束 + 柔和焦外光斑
    img = ramp(ys / h, [(0.0, '#1D1E22'), (0.5, '#2A2C32'), (1.0, '#16171A')])
    # 斜向百叶窗明暗条纹（柔化边缘）
    slats = np.sin((xs * 0.42 + ys * 0.90) * 2.0 * math.pi / 155.0)
    slats = np.clip((slats + 0.2) * 1.4, 0.0, 1.0)
    slats = blur_arr(slats * 255.0, 10.0) / 255.0
    img = blend(img, '#D8DCE3', slats * 0.58)
    # 焦外光斑（Bokeh）
    for cx, cy, r, op in [(220, 210, 180, 0.36), (860, 320, 220, 0.30), (140, 1280, 200, 0.32), (890, 1380, 250, 0.38)]:
        img = screen(img, '#F2F4F8', glow(BG, cx, cy, r, 1.8) * op)
    # 暗房小红标点缀
    mark = svg_rgba('''
        <rect x="48" y="64" width="10" height="44" fill="#D93829" fill-opacity="0.88"/>
        <circle cx="1012" cy="1530" r="8" fill="#D93829" fill-opacity="0.88"/>
    ''', BG)
    img = over(img, mark)
    bg = save(grain(img, 9.5, 121), 'bg', 'silver')

    # 胶带：35mm 黑白胶片齿孔 + 暗房灰阶曝光试条 + 红色帧标
    sprockets = ''.join(f'<rect x="{x}" y="6" width="11" height="8" rx="2" fill="#F2F3F5"/><rect x="{x}" y="82" width="11" height="8" rx="2" fill="#F2F3F5"/>' for x in range(4, 100, 20))
    steps = ''.join(f'<rect x="{i * 20}" y="20" width="20" height="56" fill="#{v:02x}{v:02x}{v:02x}"/>' for i, v in enumerate([38, 76, 126, 178, 224]))
    tape = tape_svg('#191A1E', 100, f'''
        {steps}
        {sprockets}
        <circle cx="90" cy="48" r="3.5" fill="#D93829"/>
    ''', seed=122, grain_amt=4.2)
    save(tape, 'tape', 'silver')

    # 封面：横向黑白银盐斜向百叶窗光束
    xs, ys = grid(COVER)
    cov = ramp(xs / COVER[0], [(0.0, '#1B1C20'), (0.5, '#2B2D33'), (1.0, '#151619')])
    sl = np.sin((xs * 0.35 + ys * 0.94) * 2.0 * math.pi / 135.0)
    sl = blur_arr(np.clip((sl + 0.15) * 1.4, 0.0, 1.0) * 255.0, 8.0) / 255.0
    cov = blend(cov, '#DCE0E7', sl * 0.60)
    cov = screen(cov, '#F4F6FA', glow(COVER, 680, 180, 320, 1.8) * 0.35)
    cov = screen(cov, '#F4F6FA', glow(COVER, 1460, 220, 280, 1.8) * 0.30)
    cov = over(cov, svg_rgba('<rect x="84" y="176" width="10" height="48" fill="#D93829" fill-opacity="0.9"/>', COVER))
    save(grain(cov, 8.5, 123), 'cover', 'silver')
    return bg


META = [
    ('prism',      '棱镜',   'Prism',         '#15171E', '#FAF9F6', ''),
    ('celadon',    '影青',   'Celadon',       '#16332D', '#EEF5F2', 'serif'),
    ('caustics',   '焦散',   'Caustics',      '#0B2E46', '#F4FBFC', 'rounded'),
    ('obsidian',   '金缮',   'Kintsugi',      '#F3EADB', '#141312', 'serif'),
    ('dichroic',   '虹彩',   'Dichroic',      '#EEF2FF', '#111326', 'mono'),
    ('terracotta', '赤陶',   'Terracotta',    '#3F1C14', '#FDF4EC', ''),
    ('aurora',     '极光',   'Aurora',        '#E3FAF4', '#0C1922', ''),
    ('vermilion',  '朱漆',   'Lacquer',       '#3B100E', '#FBF5EC', 'serif'),
    ('cobalt',     '群青',   'Cobalt',        '#F7F9FF', '#12277A', 'mono'),
    ('matcha',     '枯山水', 'Karesansui',    '#182918', '#DCE8D3', 'serif'),
    ('amber',      '琥珀',   'Amber',         '#FCEBD0', '#23150B', 'rounded'),
    ('silver',     '银盐',   'Silver Halide', '#141518', '#F2F3F5', 'mono'),
]


if __name__ == '__main__':
    targets = sys.argv[1:] or [m[0] for m in META]
    meta_map = {m[0]: m for m in META}
    for name in targets:
        bg = STYLES[name]()
        _, zh, en, ink, paper, font = meta_map[name]
        ratio = wcag_contrast(ink, paper)
        print(f'{name:12s} {zh:4s} top=0x{top_hex(bg):06x} contrast={ratio:.2f}:1')
