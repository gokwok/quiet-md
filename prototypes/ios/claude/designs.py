"""Claude 的十二套样式的画法（和 Fable 一起定的方案）。每套一个函数，画背景、胶带、封面；SPEC 是给预览脚本看的配色。

背景 1066×1600：图库卡片露出中间那块正方形（y 267–1333）四周，文档页露出顶上一条（y < 210，x 163–903）和两侧窄条，
所以每张都是满幅的图，顶上和四周都有看头。胶带的花纹周期都能整除 800，左右两端接得上。
"""
import math
import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from draw import (BG, TAPE, COVER, rgb, canvas, grid, ramp, blend, grain, noise, svg, over, halftone, save, style)

SS = 3  # 形状先放大这么多倍画，再缩回来，边缘是平滑的


def mask(size, fn):
    """fn(d, k) 在放大 k 倍的灰度图上画（坐标自己乘 k），返回 0–1 的遮罩。"""
    w, h = size
    im = Image.new('L', (w * SS, h * SS), 0)
    fn(ImageDraw.Draw(im), SS)
    return np.asarray(im.resize((w, h), Image.LANCZOS), np.float32) / 255


def multiply(img, color, m):
    c = rgb(color) / 255
    m = m[..., None]
    return img * (1 - m + m * c)


def disc(d, k, x, y, r, **kw):
    d.ellipse(((x - r) * k, (y - r) * k, (x + r) * k, (y + r) * k), **kw)


def poly(d, k, pts, **kw):
    d.polygon([(x * k, y * k) for x, y in pts], **kw)


def ring_band(dist, pitch, width):
    """同心圈：dist 是到圆心的距离，返回圈线的遮罩（抗锯齿）。"""
    f = np.abs((dist / pitch) % 1 - 0.5) * pitch  # 到圈中线的距离
    return np.clip(width / 2 - (pitch / 2 - f) + 0.5, 0, 1)


def isolines(h, levels, width, bold_every=5):
    v = h * levels
    gy, gx = np.gradient(v)
    g = np.hypot(gx, gy) + 1e-6
    f = v - np.round(v)
    d = np.abs(f) / g
    bold = (np.round(v).astype(int) % bold_every) == 0
    wd = np.where(bold, width * 2, width)
    return np.clip(wd / 2 - d + 0.5, 0, 1)


def wrap_x(period, width):
    """胶带上一个花纹在每个周期都画一次，两端多画一个，裁掉的部分正好接上。"""
    return range(-period, width + period, period)


SPEC = {}


def spec(name, **kw):
    SPEC[name] = {"bg": name, "tape": name, **kw}


# ---------------------------------------------------------------- 1 孔版

spec('riso', zh='孔版', en='Riso Press', text='#1B2A8F', paper='#FFF3E0', font='', cover='riso')
PINK, YEL, BLUE = '#FF4F7B', '#FFD23F', '#2D4BD8'


@style
def riso():
    img = canvas(BG, '#FFF3E0')
    xs, ys = grid(BG)
    # 粉版：上左一大团、下右一团，团里是网点（越往下点越大）
    pink = mask(BG, lambda d, k: (disc(d, k, 250 + 6, 180 - 4, 380, fill=255), disc(d, k, 930 + 6, 1240 - 4, 300, fill=255)))
    dots = halftone(BG, 0.25 + 0.6 * ((ys % 800) / 800), 14, math.radians(45))
    img = multiply(img, PINK, pink * (0.35 + 0.65 * dots))
    yel = mask(BG, lambda d, k: (disc(d, k, 800 - 5, 260 + 7, 320, fill=255), disc(d, k, 120 - 5, 1080 + 7, 260, fill=255), disc(d, k, 560, 1500, 200, fill=255)))
    img = multiply(img, YEL, yel)
    def blue_marks(d, k):
        for x, y, r, s in ((560 + 3, 60 + 3, 260, 70), (300, 1350, 180, 52), (980, 620, 140, 40)):
            disc(d, k, x, y, r, outline=255, width=s * k)
        # 左右两边的斜纹
        for i in range(-40, 60):
            y0 = i * 60
            for x0, x1 in ((0, 70), (996, 1066)):
                poly(d, k, [(x0, y0), (x1, y0 - 70), (x1, y0 - 40), (x0, y0 + 30)], fill=255)
    img = multiply(img, BLUE, mask(BG, blue_marks) * 0.92)
    return save(grain(img, 8, 1), 'bg', 'riso'), riso_tape(), riso_cover()


def riso_tape():
    img = canvas(TAPE, YEL)
    xs, ys = grid(TAPE)
    r = 3 + 9 * (0.5 + 0.5 * np.sin(2 * np.pi * xs / 200))
    u, v = (xs - 12.5) / 25, (ys - 21) / 25
    d = np.hypot(u - np.round(u), v - np.round(v)) * 25
    img = multiply(img, PINK, np.clip(r - d + 0.5, 0, 1))
    lines = mask(TAPE, lambda d, k: [d.rectangle((0, (y + o) * k, 800 * k, (y + o + 6) * k), fill=255) for y, o in ((24, 3), (168, -3))])
    img = multiply(img, BLUE, lines)
    return save(grain(img, 6, 2), 'tape', 'riso')


def riso_cover():
    img = canvas(COVER, '#FFF3E0')
    xs, ys = grid(COVER)
    dots = halftone(COVER, 0.3 + 0.5 * ys / 400, 13, math.radians(45))
    for i, x in enumerate(range(200, 2000, 400)):
        y = 200 + (60 if i % 2 else -60)
        m = mask(COVER, lambda d, k: disc(d, k, x, y, 230, fill=255))
        c = (PINK, YEL, BLUE)[i % 3]
        img = multiply(img, c, m * ((0.35 + 0.65 * dots) if c == PINK else (0.85 if c == BLUE else 1)))
    return save(grain(img, 7, 3), 'cover', 'riso')


# ---------------------------------------------------------------- 2 等高线

spec('contour', zh='等高线', en='Contour', text='#F4FFD6', paper='#0E3B2E', font='mono', cover='contour')


def height_field(size, peaks, seed):
    xs, ys = grid(size)
    h = np.zeros_like(xs)
    for x, y, s, a in peaks:
        h += a * np.exp(-((xs - x) ** 2 + (ys - y) ** 2) / (2 * s * s))
    return h + 0.35 * noise(size, 300, seed, 3)


@style
def contour():
    img = canvas(BG, '#072219')
    peaks = [(760, 210, 260, 1.0), (180, 520, 300, 0.7), (930, 900, 240, 0.8), (260, 1300, 320, 0.9), (700, 1500, 200, 0.5), (520, 760, 380, 0.35)]
    h = height_field(BG, peaks, 7)
    img = blend(img, '#C8FF2E', isolines(h, 22, 2))
    def marks(d, k):
        x, y = 760, 210
        poly(d, k, [(x, y - 16), (x - 14, y + 10), (x + 14, y + 10)], fill=255)
        for a in range(0, 360, 20):
            t0, t1 = math.radians(a), math.radians(a + 11)
            d.arc(((x - 60) * k, (y - 60) * k, (x + 60) * k, (y + 60) * k), a, a + 11, fill=255, width=3 * k)
    img = blend(img, '#FF6A2B', mask(BG, marks))
    ticks = mask(BG, lambda d, k: [d.rectangle((x0 * k, y * k, x1 * k, (y + 3) * k), fill=255) for y in range(20, 1600, 40) for x0, x1 in ((0, 18 if y % 200 == 20 else 10), (1066 - (18 if y % 200 == 20 else 10), 1066))])
    img = blend(img, '#5E8F3A', ticks)
    return save(grain(img, 5, 4), 'bg', 'contour'), contour_tape(), contour_cover()


def contour_tape():
    img = canvas(TAPE, '#FF6A2B')
    def marks(d, k):
        for x in range(0, 800, 10):
            t = 70 if x % 100 == 0 else 44 if x % 50 == 0 else 20
            d.rectangle((x * k, 0, (x + 2) * k, t * k), fill=255)
        for x in wrap_x(100, 800):
            c = x + 50
            poly(d, k, [(c, 126), (c - 8, 146), (c + 8, 146)], fill=255)
    img = blend(img, '#10140F', mask(TAPE, marks))
    return save(grain(img, 4, 5), 'tape', 'contour')


def contour_cover():
    img = canvas(COVER, '#072219')
    peaks = [(300, 120, 160, 0.9), (900, 300, 200, 1.0), (1500, 100, 180, 0.8), (1850, 330, 150, 0.6)]
    h = height_field(COVER, peaks, 11)
    xs, _ = grid(COVER)
    line = ramp(xs / 2000, [(0, '#C8FF2E'), (1, '#FF6A2B')])
    img = blend(img, line, isolines(h, 14, 2.5))
    def route(d, k):
        pts = [(80, 330), (420, 250), (700, 290), (1000, 160), (1350, 210), (1650, 90), (1950, 140)]
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            n = int(math.hypot(x1 - x0, y1 - y0) / 22)
            for i in range(0, n, 2):
                a, b = i / n, min(1, (i + 1) / n)
                d.line(((x0 + (x1 - x0) * a) * k, (y0 + (y1 - y0) * a) * k, (x0 + (x1 - x0) * b) * k, (y0 + (y1 - y0) * b) * k), fill=255, width=3 * k)
    img = blend(img, '#F4FFD6', mask(COVER, route))
    return save(grain(img, 4, 6), 'cover', 'contour')


# ---------------------------------------------------------------- 3 蓝晒

spec('cyanotype', zh='蓝晒', en='Cyanotype', text='#173F75', paper='#F4F0E6', font='mono', cover='cyanotype')
PRUSSIAN = '#1B4A86'


def leaf(cx, cy, ang, length, width, tip=1.0):
    """一片尖头的叶子（多边形），从 (cx, cy) 沿 ang 方向长出去。"""
    pts = []
    for i in range(25):
        t = i / 24
        pts.append((t * length, width * math.sin(math.pi * t) ** 0.8 * (1 - 0.3 * t * tip)))
    pts += [(x, -y) for x, y in reversed(pts)]
    c, s = math.cos(ang), math.sin(ang)
    return [(cx + x * c - y * s, cy + x * s + y * c) for x, y in pts]


def frond(d, k, x, y, ang, length, lw, bend=0.25, fill=255):
    """一枝蕨：弯的茎，两边交替长叶，越往梢越小。"""
    pts, a = [], ang
    n = 40
    for i in range(n + 1):
        pts.append((x, y))
        x += math.cos(a) * length / n
        y += math.sin(a) * length / n
        a += bend / n
    d.line([(px * k, py * k) for px, py in pts], fill=fill, width=max(1, int(lw * 0.18 * k)))
    for i in range(3, n, 2):
        px, py = pts[i]
        t = i / n
        dx, dy = pts[i + 1][0] - px, pts[i + 1][1] - py
        sa = math.atan2(dy, dx)
        side = 1 if (i // 2) % 2 else -1
        ll = lw * (1.15 - t) * 1.7
        poly(d, k, leaf(px, py, sa + side * 1.05, ll, ll * 0.24), fill=fill)


def sunprint(size, draw_fn, seed):
    """蓝晒：叠了几层的白剪影，各层曝光不同，边缘略化开。"""
    w, h = size
    m = np.zeros((h, w), np.float32)
    for alpha, fn in draw_fn:
        layer = mask(size, fn)
        m = 1 - (1 - m) * (1 - alpha * layer)
    m = np.asarray(Image.fromarray((m * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.3)), np.float32) / 255
    blue = ramp(noise(size, 420, seed, 3), [(0, '#0F2F5E'), (0.5, PRUSSIAN), (1, '#2A62A8')])
    img = blend(blue, '#EEF3F8', m * 0.94)
    return grain(img, 6, seed)


@style
def cyanotype():
    layers = [
        (0.9, lambda d, k: frond(d, k, -20, 120, -0.15, 620, 70, 0.5)),
        (0.75, lambda d, k: frond(d, k, 1090, 40, 2.6, 520, 60, -0.4)),
        (0.85, lambda d, k: frond(d, k, -30, 760, -0.6, 420, 62, 0.6)),
        (0.7, lambda d, k: frond(d, k, 1100, 980, 3.6, 480, 66, 0.3)),
        (0.9, lambda d, k: frond(d, k, 60, 1640, -1.1, 600, 74, 0.45)),
        (0.6, lambda d, k: frond(d, k, 1000, 1620, -2.2, 520, 58, -0.5)),
        (0.55, lambda d, k: frond(d, k, 460, -40, 1.9, 300, 50, -0.6)),
        (0.8, lambda d, k: [poly(d, k, leaf(x, y, a, 70, 22), fill=255) for x, y, a in ((880, 360, 2.4), (930, 420, 2.0), (140, 1100, 0.6), (190, 1040, 1.0), (980, 1400, 3.5), (330, 260, 1.2))]),
    ]
    img = sunprint(BG, layers, 21)
    return save(img, 'bg', 'cyanotype'), cyanotype_tape(), cyanotype_cover()


def cyanotype_tape():
    def sprigs(d, k):
        for x in wrap_x(400, 800):
            frond(d, k, x + 10, 150, -0.12, 220, 34, 0.15)
            frond(d, k, x + 230, 40, 0.18, 190, 30, -0.1)
            poly(d, k, leaf(x + 205, 140, -0.4, 40, 12), fill=255)
    img = sunprint(TAPE, [(0.9, sprigs)], 22)
    return save(img, 'tape', 'cyanotype')


def cyanotype_cover():
    layers = [
        (0.9, lambda d, k: frond(d, k, -40, 330, -0.2, 900, 80, 0.25)),
        (0.7, lambda d, k: frond(d, k, 2040, 60, 2.95, 820, 70, -0.25)),
        (0.8, lambda d, k: frond(d, k, 980, 420, -1.3, 380, 56, 0.6)),
        (0.6, lambda d, k: [poly(d, k, leaf(x, y, a, 80, 24), fill=255) for x, y, a in ((700, 80, 0.4), (760, 120, 0.9), (1500, 330, -0.6), (1560, 290, -0.2))]),
    ]
    return save(sunprint(COVER, layers, 23), 'cover', 'cyanotype')


# ---------------------------------------------------------------- 4 墨流

spec('suminagashi', zh='墨流', en='Suminagashi', text='#F2EAD8', paper='#14161C', font='serif', doodle=True, tape=None)


@style
def suminagashi():
    xs, ys = grid(BG)
    warp = (noise(BG, 260, 31, 2) - 0.5) * 2 * 140 + (noise(BG, 90, 32, 2) - 0.5) * 30
    u = np.hypot(xs + 160, ys + 120) + warp
    cols = ['#0B0C10', '#F2EAD8', '#0B0C10', '#2A2D36', '#0B0C10', '#F2EAD8', '#2A2D36', '#0B0C10', '#E8452C']
    band = (np.floor(u / 34).astype(int)) % 9
    pal = np.stack([rgb(c) for c in cols])
    img = pal[band]
    # 第二滴墨：右下角又落一滴，圈套圈地推开前一滴的纹
    u2 = np.hypot(xs - 900, ys - 1350) + warp * 0.8
    inner = u2 < 330
    band2 = (np.floor(u2 / 30).astype(int)) % 4
    pal2 = np.stack([rgb(c) for c in ('#F2EAD8', '#0B0C10', '#2A2D36', '#0B0C10')])
    img = np.where(inner[..., None], pal2[band2], img)
    im = Image.fromarray(img.astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.9))
    img = grain(np.asarray(im, np.float32), 4, 33)
    return save(img, 'bg', 'suminagashi'),


# ---------------------------------------------------------------- 5 水磨石

spec('terrazzo', zh='水磨石', en='Terrazzo', text='#5A1F3D', paper='#FFF6EC', font='rounded')


def chip(rng, x, y, r):
    n = rng.integers(4, 8)
    a0 = rng.random() * math.tau
    angs = sorted(a0 + rng.random() * math.tau for _ in range(n))
    return [(x + math.cos(a) * r * (0.7 + 0.3 * rng.random()), y + math.sin(a) * r * (0.7 + 0.3 * rng.random())) for a in angs]


def poisson(rng, w, h, spacing, tries=6000):
    pts = []
    for _ in range(tries):
        x, y = rng.random() * w, rng.random() * h
        if all((x - a) ** 2 + (y - b) ** 2 > spacing ** 2 for a, b in pts[-400:]):
            pts.append((x, y))
    return pts


@style
def terrazzo():
    rng = np.random.default_rng(41)
    img = canvas(BG, '#F7B8C4')
    img = blend(img, '#F2A5B4', noise(BG, 200, 42, 3) * 0.6)
    palette = ['#1F3FD0'] * 3 + ['#F2B705'] * 3 + ['#0E6B4F'] * 2 + ['#FFFFFF'] * 3 + ['#16121A']
    pts = poisson(rng, 1066, 1600, 50)
    layers = {c: [] for c in set(palette)}
    for x, y in pts:
        r = 10 + rng.random() * 36
        if y < 420: r *= 1.5
        layers[palette[rng.integers(len(palette))]].append(chip(rng, x, y, r))
    for _ in range(900):
        layers[palette[rng.integers(len(palette))]].append(chip(rng, rng.random() * 1066, rng.random() * 1600, 2 + rng.random() * 3))
    for c, polys in layers.items():
        img = blend(img, c, mask(BG, lambda d, k: [poly(d, k, p, fill=255) for p in polys]))
    return save(grain(img, 5, 43), 'bg', 'terrazzo'), terrazzo_tape()


def terrazzo_tape():
    rng = np.random.default_rng(44)
    img = canvas(TAPE, '#9BE7C4')
    palette = ['#1F3FD0', '#FF5A5F', '#FFFFFF', '#16121A', '#1F3FD0', '#FF5A5F', '#FFFFFF']
    chips = []
    for x, y in poisson(rng, 400, 192, 38):
        r = 8 + rng.random() * 20
        c = palette[rng.integers(len(palette))]
        p = chip(rng, x, y, r)
        for off in (-400, 0, 400, 800):
            chips.append((c, [(a + off, b) for a, b in p]))
    for c in set(palette):
        img = blend(img, c, mask(TAPE, lambda d, k: [poly(d, k, p, fill=255) for cc, p in chips if cc == c]))
    return save(grain(img, 4, 45), 'tape', 'terrazzo')


# ---------------------------------------------------------------- 6 日冕

spec('sunburst', zh='日冕', en='Sunburst', text='#2A0E00', paper='#FFD400', font='rounded', cover='sunburst')


def rays(size, cx, cy, n, a, b, halftone_on=None):
    xs, ys = grid(size)
    ang = np.arctan2(ys - cy, xs - cx)
    r = np.hypot(xs - cx, ys - cy)
    f = ang / (2 * np.pi) * n
    edge = np.abs(f - np.round(f))  # 到楔形边的角距离（以楔为单位）
    side = (np.floor(f).astype(int) % 2) == 0
    aa = np.clip(edge * (2 * np.pi / n) * r + 0.5, 0, 1)  # 边上抗锯齿
    img = np.where(side[..., None], rgb(a), rgb(b))
    other = np.where(side[..., None], rgb(b), rgb(a))
    img = img * aa[..., None] + other * (1 - aa[..., None]) * 0.5 + img * (1 - aa[..., None]) * 0.5
    if halftone_on is not None:
        dots = halftone(size, np.clip(r / 1500, 0, 1) * 0.55 + 0.05, 12, 0.35)
        img = blend(img, halftone_on, dots * side * aa)
    return img


@style
def sunburst():
    img = rays(BG, 533, -60, 36, '#FF3D2E', '#FF8FB1', '#C81E12')
    sun = mask(BG, lambda d, k: disc(d, k, 533, -60, 264, fill=255))
    img = blend(img, '#2A0E00', sun)
    img = blend(img, '#FFF3E0', mask(BG, lambda d, k: disc(d, k, 533, -60, 250, fill=255)))
    return save(grain(img, 5, 51), 'bg', 'sunburst'), sunburst_tape(), sunburst_cover()


def sunburst_tape():
    img = canvas(TAPE, '#FF3D2E')
    def suns(d, k):
        for x in wrap_x(160, 800):
            c = x + 80
            disc(d, k, c, 192, 62, fill=255)
            for i in range(9):
                a = math.pi + math.pi * (i + 0.5) / 9
                x0, y0 = c + math.cos(a) * 76, 192 + math.sin(a) * 76
                x1, y1 = c + math.cos(a) * 106, 192 + math.sin(a) * 106
                d.line((x0 * k, y0 * k, x1 * k, y1 * k), fill=255, width=8 * k)
    img = blend(img, '#FFF3E0', mask(TAPE, suns))
    img = blend(img, '#2A0E00', mask(TAPE, lambda d, k: d.rectangle((0, 14 * k, 800 * k, 20 * k), fill=255)))
    return save(grain(img, 4, 52), 'tape', 'sunburst')


def sunburst_cover():
    img = rays(COVER, 1000, 400, 24, '#FF3D2E', '#FF8FB1')
    img = blend(img, '#2A0E00', mask(COVER, lambda d, k: disc(d, k, 1000, 400, 312, fill=255)))
    img = blend(img, '#FFD400', mask(COVER, lambda d, k: disc(d, k, 1000, 400, 300, fill=255)))
    img = blend(img, '#FFF3E0', mask(COVER, lambda d, k: [d.rounded_rectangle(((x - 210) * k, (y - 28) * k, (x + 210) * k, (y + 28) * k), 28 * k, fill=255) for x, y in ((300, 250), (1560, 180))]))
    return save(grain(img, 4, 53), 'cover', 'sunburst')


# ---------------------------------------------------------------- 7 青海波

spec('tideline', zh='青海波', en='Tideline', text='#0B2A5B', paper='#F3EEDD', font='serif', cover='tideline')


def seigaiha(size, r, dy, ground, line, start_y=-1, width=3):
    w, h = size
    def scales(d, k):
        row, y = 0, start_y if start_y >= 0 else -r
        while y < h + r:
            off = 0 if row % 2 == 0 else r
            x = -2 * r + off
            while x < w + 2 * r:
                disc(d, k, x, y, r, fill=ground)
                for f in (1, 0.76, 0.52, 0.28):
                    disc(d, k, x, y, r * f - width / 2, outline=line, width=width * k)
                x += 2 * r
            y += dy
            row += 1
    im = Image.new('L', (w * SS, h * SS), ground)
    scales(ImageDraw.Draw(im), SS)
    return np.asarray(im.resize((w, h), Image.LANCZOS), np.float32) / 255


@style
def tideline():
    m = seigaiha(BG, 66, 33, 0, 255)
    img = blend(canvas(BG, '#123A8C'), '#F3EEDD', m)
    _, ys = grid(BG)
    img = blend(img, '#0B2A5B', np.clip((300 - ys) / 140, 0, 1))
    img = blend(img, '#E8402A', mask(BG, lambda d, k: disc(d, k, 790, 150, 110, fill=255)))
    return save(grain(img, 4, 61), 'bg', 'tideline'), tideline_tape(), tideline_cover()


def tideline_tape():
    m = seigaiha(TAPE, 50, 25, 0, 255, width=4)
    img = blend(canvas(TAPE, '#E8402A'), '#FFF3E0', m)
    return save(grain(img, 3, 62), 'tape', 'tideline')


def tideline_cover():
    img = canvas(COVER, '#F3EEDD')
    rng = np.random.default_rng(63)
    img = blend(img, '#E8402A', mask(COVER, lambda d, k: disc(d, k, 1500, 150, 120, fill=255)))
    bars = [(rng.random() * 1700, 50 + i * 30, 180 + rng.random() * 380) for i in range(5)]
    img = blend(img, '#0B2A5B', mask(COVER, lambda d, k: [d.rounded_rectangle((x * k, y * k, (x + l) * k, (y + 6) * k), 3 * k, fill=255) for x, y, l in bars]))
    m = seigaiha(COVER, 60, 30, 0, 255, start_y=270)
    wave = blend(canvas(COVER, '#123A8C'), '#F3EEDD', m)
    _, ys = grid(COVER)
    sea = mask(COVER, lambda d, k: [disc(d, k, x, 270, 60, fill=255) for x in range(-120, 2121, 120)] + [d.rectangle((0, 270 * k, 2000 * k, 400 * k), fill=255)])
    img = img * (1 - sea[..., None]) + wave * sea[..., None]
    return save(grain(img, 3, 64), 'cover', 'tideline')


# ---------------------------------------------------------------- 8 荧屏

spec('phosphor', zh='荧屏', en='Phosphor', text='#7CFF6B', paper='#07140B', font='mono', faded=True, cover='phosphor')


def lissajous(d, k, cx, cy, ax, ay, a, b, phase, width):
    pts = []
    for i in range(2001):
        t = i / 2000 * math.tau
        pts.append(((cx + ax * math.sin(a * t + phase)) * k, (cy + ay * math.sin(b * t)) * k))
    d.line(pts, fill=255, width=int(width * k), joint='curve')


def phosphor_glow(size, fn, blur):
    m = mask(size, fn)
    g = np.asarray(Image.fromarray((m * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(blur)), np.float32) / 255
    return m, g


def scanlines(img, every=3, k=0.35):
    img[::every] *= (1 - k)
    return img


@style
def phosphor():
    img = canvas(BG, '#020503')
    xs, ys = grid(BG)
    grid_m = (((xs % 53) < 2) & ((ys % 6) < 2)) | (((ys % 53) < 2) & ((xs % 6) < 2))
    img = blend(img, '#1E5A2A', grid_m * 0.9)
    traces = lambda d, k: [lissajous(d, k, 533, 220, 430, 170, a, b, p, 3) for a, b, p in ((3, 2, 0.4), (5, 4, 1.1), (3, 4, 2.0))]
    m, g = phosphor_glow(BG, traces, 12)
    img = blend(img, '#2BD94A', g * 0.6)
    img = blend(img, '#7CFF6B', m)
    # 下面再一组暗一点的轨迹（卡片中间会露出来）
    low = lambda d, k: [lissajous(d, k, 533, 1180, 470, 230, a, b, p, 2.5) for a, b, p in ((1, 2, 0.3), (2, 3, 1.5))]
    m2, g2 = phosphor_glow(BG, low, 10)
    img = blend(img, '#2BD94A', g2 * 0.6)
    img = blend(img, '#7CFF6B', m2 * 0.9)
    v = np.hypot((xs - 533) / 900, (ys - 800) / 1200)
    img = img * np.clip(1.2 - v * 0.45, 0.55, 1)[..., None]
    img = scanlines(img)
    return save(grain(img, 4, 71), 'bg', 'phosphor'), phosphor_tape(), phosphor_cover()


def phosphor_tape():
    img = canvas(TAPE, '#07140B')
    xs, ys = grid(TAPE)
    img = blend(img, '#1E5A2A', (((xs % 40) < 2) & ((ys % 5) < 2)) | (((ys % 40) < 2) & ((xs % 5) < 2)))
    def trace(d, k):
        pts = []
        for x in range(-200, 1001):
            ph = x % 200
            y = 96 - 50 * math.sin(math.tau * ph / 100) if ph < 100 else (96 - 50 if (ph - 100) < 50 else 96 + 50)
            pts.append((x * k, y * k))
        d.line(pts, fill=255, width=5 * k, joint='curve')
    m, g = phosphor_glow(TAPE, trace, 8)
    img = blend(img, '#2BD94A', g * 0.7)
    img = blend(img, '#9BFF8C', m)
    return save(scanlines(img), 'tape', 'phosphor')


def phosphor_cover():
    img = canvas(COVER, '#020503')
    for dx, col, a in ((6, '#FF4FD8', 0.35), (0, '#7CFF6B', 1)):
        m, g = phosphor_glow(COVER, lambda d, k: lissajous(d, k, 1000 + dx, 200, 900, 160, 5, 6, 0.5, 4), 10)
        img = blend(img, col, g * 0.5 * a)
        img = blend(img, col, m * a)
    return save(scanlines(img), 'cover', 'phosphor')


# ---------------------------------------------------------------- 9 构成

spec('primary', zh='构成', en='Primary', text='#111111', paper='#F2EBDD', font='', cover='primary')
RED, PBLUE, PYEL, INK = '#E4201B', '#1D3FBF', '#F5C400', '#111111'


@style
def primary():
    img = canvas(BG, '#F2EBDD')
    layers = [
        (PYEL, lambda d, k: (poly(d, k, [(640, 40), (920, 430), (500, 430)], fill=255), disc(d, k, 1066, 760, 230, fill=255), d.rectangle((0, 1180 * k, 300 * k, 1240 * k), fill=255))),
        (PBLUE, lambda d, k: (disc(d, k, 1066, 0, 420, fill=255), d.pieslice((-330 * k, 700 * k, 330 * k, 1360 * k), 270, 90, fill=255), d.rectangle((760 * k, 1300 * k, 1066 * k, 1600 * k), fill=255))),
        (RED, lambda d, k: (disc(d, k, 220, 200, 330, fill=255), d.rectangle((800 * k, 1020 * k, 1066 * k, 1110 * k), fill=255), disc(d, k, 300, 1560, 220, fill=255))),
        (INK, lambda d, k: (d.rectangle((0, 450 * k, 1066 * k, 476 * k), fill=255), d.rectangle((0, 1270 * k, 760 * k, 1290 * k), fill=255), d.line((660 * k, 520 * k, 1066 * k, 980 * k), fill=255, width=16 * k), [d.rectangle((0, y * k, 50 * k, (y + 50) * k), fill=255) for y in range(500, 1180, 100)])),
        ('#F2EBDD', lambda d, k: disc(d, k, 1066, 0, 230, fill=255)),
        (RED, lambda d, k: [disc(d, k, 1041, y, 16, fill=255) for y in range(1150, 1300, 60)]),
    ]
    for c, fn in layers:
        img = blend(img, c, mask(BG, fn))
    return save(grain(img, 5, 81), 'bg', 'primary'), primary_tape(), primary_cover()


def primary_tape():
    img = canvas(TAPE, '#F2EBDD')
    def shapes(fill_kind):
        def fn(d, k):
            for x in wrap_x(200, 800):
                if fill_kind == RED: disc(d, k, x + 40, 96, 52, fill=255)
                if fill_kind == PBLUE: poly(d, k, [(x + 100 + 60 * math.cos(a), 96 + 60 * math.sin(a)) for a in (0, math.pi / 2, math.pi, 1.5 * math.pi)], fill=255)
                if fill_kind == PYEL: poly(d, k, [(x + 160, 46), (x + 210, 140), (x + 110, 140)], fill=255)
                if fill_kind == INK: d.rectangle((0, 0, 800 * k, 10 * k), fill=255); d.rectangle((0, 182 * k, 800 * k, 192 * k), fill=255)
        return fn
    for c in (PBLUE, RED, PYEL, INK):
        img = blend(img, c, mask(TAPE, shapes(c)))
    return save(grain(img, 4, 82), 'tape', 'primary')


def primary_cover():
    img = canvas(COVER, PBLUE)
    img = blend(img, INK, mask(COVER, lambda d, k: d.line((0, 400 * k, 2000 * k, 0), fill=255, width=30 * k)))
    img = blend(img, PYEL, mask(COVER, lambda d, k: disc(d, k, 500, 400, 200, fill=255)))
    sq = [(1100 + x * math.cos(0.26) - y * math.sin(0.26), 200 + x * math.sin(0.26) + y * math.cos(0.26)) for x, y in ((-130, -130), (130, -130), (130, 130), (-130, 130))]
    img = blend(img, RED, mask(COVER, lambda d, k: poly(d, k, sq, fill=255)))
    img = blend(img, '#F2EBDD', mask(COVER, lambda d, k: disc(d, k, 1650, 200, 150, outline=255, width=40 * k)))
    return save(grain(img, 4, 83), 'cover', 'primary')


# ---------------------------------------------------------------- 10 票根

spec('ticket', zh='票根', en='Ticket Stub', text='#3B0A14', paper='#FF9EC4', font='serif')
FELT = '#00754A'


@style
def ticket():
    img = canvas(BG, FELT)
    img = blend(img, '#00603C', noise(BG, 160, 91, 4) * 0.55)
    img = grain(img, 7, 92, mono=False)
    return save(img, 'bg', 'ticket'), ticket_tape()


def star(cx, cy, r):
    return [(cx + (r if i % 2 == 0 else r * 0.42) * math.cos(-math.pi / 2 + i * math.pi / 5), cy + (r if i % 2 == 0 else r * 0.42) * math.sin(-math.pi / 2 + i * math.pi / 5)) for i in range(10)]


def ticket_tape():
    img = canvas(TAPE, FELT)
    def body(d, k):
        for x in wrap_x(200, 800):
            d.rounded_rectangle(((x + 8) * k, 20 * k, (x + 192) * k, 172 * k), 14 * k, fill=255)
            for nx in (x + 8, x + 192):
                disc(d, k, nx, 96, 16, fill=0)
    def ink(d, k):
        for x in wrap_x(200, 800):
            d.rounded_rectangle(((x + 20) * k, 32 * k, (x + 180) * k, 160 * k), 8 * k, outline=255, width=6 * k)
            poly(d, k, star(x + 100, 96, 30), fill=255)
            for i in range(4):
                for bx in (x + 34 + i * 9, x + 140 + i * 9):
                    d.rectangle((bx * k, 83 * k, (bx + 5) * k, 109 * k), fill=255)
    def perf(d, k):
        for x in wrap_x(200, 800):
            for y in range(28, 170, 12):
                disc(d, k, x + 8, y, 2.2, fill=255)
    img = blend(img, '#FFF3E0', mask(TAPE, body))
    img = blend(img, RED, mask(TAPE, ink))
    img = blend(img, FELT, mask(TAPE, perf))
    return save(grain(img, 4, 93), 'tape', 'ticket')


# ---------------------------------------------------------------- 11 夜曲

spec('nocturne', zh='夜曲', en='Nocturne', text='#FFE9A8', paper='#3A0D12', font='serif', doodle=True, tape=None)
GOLD = '#E6B450'


@style
def nocturne():
    _, ys = grid(BG)
    img = ramp(ys / 1600, [(0, '#1C0508'), (1, '#4A0F18')])
    rng = np.random.default_rng(101)
    def staves(d, k):
        for y0, ph, amp in ((120, 0.3, 90), (330, 2.1, 70), (760, 4.0, 110), (1160, 1.0, 80), (1480, 3.1, 90)):
            f = lambda x: y0 + amp * math.sin(math.tau * x / 1066 + ph)
            for j in range(5):
                d.line([(x * k, (f(x) + j * 16) * k) for x in range(-10, 1080, 6)], fill=255, width=2 * k)
            heads = sorted(rng.uniform(30, 1036, 14))
            for i, x in enumerate(heads):
                y = f(x) + rng.integers(0, 9) * 8
                e = [(x + 11 * math.cos(t) * math.cos(-0.35) - 8 * math.sin(t) * math.sin(-0.35), y + 11 * math.cos(t) * math.sin(-0.35) + 8 * math.sin(t) * math.cos(-0.35)) for t in np.linspace(0, math.tau, 24)]
                poly(d, k, e, fill=255)
                d.line(((x + 10) * k, y * k, (x + 10) * k, (y - 60) * k), fill=255, width=2 * k)
                if i % 3 == 1 and i + 1 < len(heads):
                    x2 = heads[i + 1]
                    if x2 - x < 140:
                        y2 = f(x2)
                        d.line(((x + 10) * k, (y - 60) * k, (x2 + 10) * k, (y2 - 60) * k), fill=255, width=6 * k)
    img = blend(img, GOLD, mask(BG, staves) * 0.9)
    dust = mask(BG, lambda d, k: [disc(d, k, rng.random() * 1066, rng.random() * 1600, 0.5 + rng.random() * 1.5, fill=255) for _ in range(160)])
    img = blend(img, GOLD, dust * 0.45)
    return save(grain(img, 5, 102), 'bg', 'nocturne'),


# ---------------------------------------------------------------- 12 莫尔

spec('moire', zh='莫尔', en='Interference', text='#001B3A', paper='#3FE0FF', font='')


@style
def moire():
    xs, ys = grid(BG)
    img = canvas(BG, '#FF2E93')
    img = blend(img, '#000000', ring_band(np.hypot(xs - 160, ys - 120), 18, 9))
    img = blend(img, '#001B3A', ring_band(np.hypot(xs - 900, ys - 1300), 19, 9) * 0.92)
    return save(grain(img, 4, 111), 'bg', 'moire'), moire_tape()


def moire_tape():
    # 品红底上两组圈：圆心在胶带上下方，周期 400 左右能接上；交叠处出干涉纹
    xs, ys = grid(TAPE)
    img = canvas(TAPE, '#FF2E93')
    for cx, cy, pitch, col in ((0, -260, 16, '#000000'), (200, 452, 17, '#001B3A')):
        d = np.min(np.stack([np.hypot(xs - (cx + o), ys - cy) for o in (-800, -400, 0, 400, 800)]), axis=0)
        img = blend(img, col, ring_band(d, pitch, 8))
    return save(grain(img, 3, 112), 'tape', 'moire')
