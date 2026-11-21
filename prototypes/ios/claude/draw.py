#!/usr/bin/env python3
"""Claude 的样式（stylepanel::claude_presets）用到的背景、胶带和封面：全部由这个脚本画出来（本项目绘制）。

    python3 prototypes/ios/claude/draw.py            # 全部重画
    python3 prototypes/ios/claude/draw.py tide moth  # 只画这几套

背景 1066×1600 放 backdrops/，胶带 800×192 放 washi/（左右两端能无缝接上），封面 2000×400 放 covers/。
随机数都有固定的种子，重画出来一模一样。
"""
import io, math, os, subprocess, sys
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


def lerp(a, b, t):
    t = np.asarray(t, dtype=np.float32)[..., None] if np.ndim(t) else t
    return a + (b - a) * t


def ramp(t, stops):
    """t（0–1 的数组）按色标 [(位置, 颜色)…] 取色。"""
    t = np.clip(t, 0, 1)
    out = np.zeros(t.shape + (3,), np.float32)
    pos = [p for p, _ in stops]
    cols = [rgb(c) for _, c in stops]
    for ch in range(3):
        out[..., ch] = np.interp(t, pos, [c[ch] for c in cols])
    return out


def blend(img, color, alpha):
    """alpha（数组或数）处盖上颜色（单色或同尺寸的图）。"""
    a = np.clip(np.asarray(alpha, np.float32), 0, 1)
    if a.ndim == 2: a = a[..., None]
    c = rgb(color) if isinstance(color, (str, int)) else color
    return img * (1 - a) + c * a


def glow(size, cx, cy, r):
    """圆心 (cx, cy)、半径 r（像素）的柔光遮罩，0–1。"""
    xs, ys = grid(size)
    d = np.sqrt((xs - cx) ** 2 + (ys - cy) ** 2) / r
    return np.clip(1 - d, 0, 1) ** 2


def grain(img, amount, seed=1, mono=True):
    rng = np.random.default_rng(seed)
    h, w, _ = img.shape
    n = rng.normal(0, amount, (h, w, 1) if mono else (h, w, 3)).astype(np.float32)
    return img + n


def noise(size, scale, seed=0, octaves=4):
    """平滑的值噪声（0–1），scale 是最大一层格子的像素。"""
    w, h = size
    rng = np.random.default_rng(seed)
    out = np.zeros((h, w), np.float32)
    amp, total = 1., 0.
    for o in range(octaves):
        s = max(2, scale / (2 ** o))
        gw, gh = int(w / s) + 3, int(h / s) + 3
        g = Image.fromarray((rng.random((gh, gw)) * 255).astype(np.uint8))
        g = g.resize((int(gw * s), int(gh * s)), Image.BICUBIC).crop((0, 0, w, h))
        out += np.asarray(g, np.float32) / 255 * amp
        total += amp
        amp *= 0.5
    return out / total


def svg(markup, size):
    """用 rsvg-convert 把 SVG 画成 RGBA 数组（0–255 浮点）。"""
    w, h = size
    doc = f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{markup}</svg>'
    png = subprocess.run(['rsvg-convert', '-w', str(w), '-h', str(h)], input=doc.encode(), capture_output=True, check=True).stdout
    return np.asarray(Image.open(io.BytesIO(png)).convert('RGBA'), np.float32)


def over(img, rgba):
    a = rgba[..., 3:4] / 255
    return img * (1 - a) + rgba[..., :3] * a


def halftone(size, value, cell, angle=0.3, color=None):
    """value（0–1 的数组，越大点越大）的半调网点遮罩。"""
    w, h = size
    xs, ys = grid(size)
    c, s = math.cos(angle), math.sin(angle)
    u, v = (xs * c + ys * s) / cell, (-xs * s + ys * c) / cell
    du, dv = u - np.round(u), v - np.round(v)
    d = np.sqrt(du * du + dv * dv)
    r = np.sqrt(np.clip(value, 0, 1)) * 0.62
    return np.clip((r - d) * cell * 0.9 + 0.5, 0, 1)


def save(img, kind, name, quality=90):
    folder = {'bg': 'backdrops', 'tape': 'washi', 'cover': 'covers'}[kind]
    path = os.path.join(HERE, folder, f'{name}.jpg')
    Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).save(path, quality=quality, optimize=True, progressive=True, subsampling=0 if kind == 'tape' else 2)
    return path


def seamless(img):
    """胶带：右端最后一段和左端交叉淡化，首尾能接上（花纹本来就按整宽重复的不需要）。"""
    h, w, _ = img.shape
    f = 48
    t = np.linspace(0, 1, f, dtype=np.float32)[None, :, None]
    img[:, w - f:] = img[:, w - f:] * (1 - t) + img[:, :f] * t
    return img


STYLES = {}


def style(fn):
    STYLES[fn.__name__] = fn
    return fn


if __name__ == '__main__':
    sys.path.insert(0, HERE)
    import draw, designs  # noqa: F401  每套的画法在 designs.py，注册到 draw.STYLES
    for n in sys.argv[1:] or list(draw.STYLES):
        print(n, *draw.STYLES[n]())
