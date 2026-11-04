"""生成 VoiceKey 的全部图标（应用、托盘，README 也直接引用这里的 128x128）。

设计：深色圆角徽章 + 麦克风。灯头用暖橙渐变（唯一彩色元素，视线焦点），
支架白色——16px 缩到托盘尺寸时仍能辨认轮廓。

用法：python app/src-tauri/icons/logo.py
依赖：Pillow（WinPython 自带）
"""

from pathlib import Path

from PIL import Image, ImageDraw

ICONS = Path(__file__).resolve().parent
ROOT = ICONS.parents[2]

S = 1024  # 超采样画布，最后缩到目标尺寸
ORANGE_TOP = (255, 160, 106)
ORANGE_BOTTOM = (216, 85, 48)
WHITE = (245, 245, 242)


def gradient(size, top, bottom):
    """竖直渐变"""
    img = Image.new("RGB", (1, size), top)
    px = img.load()
    for y in range(size):
        t = y / max(size - 1, 1)
        px[0, y] = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
    return img


def draw_mark(draw, s, stroke=1.0, color=WHITE):
    """麦克风：灯头 + 托架弧 + 支杆 + 底座。坐标按 1024 画布给，再按 s 缩放。

    stroke 放大托架/支杆/底座的线宽，托盘这种小尺寸用得上。
    """
    k = s / 1024

    def box(x0, y0, x1, y1):
        return [round(v * k) for v in (x0, y0, x1, y1)]

    capsule = box(362, 210, 662, 690)
    radius = round(150 * k)

    # 托架：接近水平起止，端头不与灯头抢视觉
    arc_r = round(240 * k)
    cx, cy = round(512 * k), round(540 * k)
    arc_w = round(50 * stroke * k)
    draw.arc([cx - arc_r, cy - arc_r, cx + arc_r, cy + arc_r], 8, 172, fill=color, width=arc_w)

    # 支杆与底座
    stem_w = round(50 * stroke * k)
    draw.rounded_rectangle(box(512 - stem_w / 2 / k, 780, 512 + stem_w / 2 / k, 862),
                           radius=round(stem_w / 2), fill=color)
    base_h = round(46 * stroke * k)
    draw.rounded_rectangle(box(396, 858, 628, 858 + base_h / k), radius=round(base_h / 2), fill=color)
    return capsule, radius


def render(size, glyph_only=False, tint=None, stroke=1.0):
    s = S
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    if not glyph_only:
        # 徽章背景：竖直深色渐变裁进圆角矩形
        bg = gradient(s, (44, 44, 39), (20, 20, 17)).resize((s, s)).convert("RGBA")
        mask = Image.new("L", (s, s), 0)
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, s - 1, s - 1], radius=round(s * 0.225), fill=255)
        img.paste(bg, (0, 0), mask)

    # 托盘：字形用中性灰，浅色/深色任务栏都可见；灯头保持品牌橙
    mark_color = (tint if tint else (124, 124, 118, 255)) if glyph_only else WHITE
    capsule, radius = draw_mark(d, s, stroke=stroke, color=mark_color)

    # 灯头填充：暖橙渐变（托盘单色版则统一 tint）
    if tint:
        d.rounded_rectangle(capsule, radius=radius, fill=tint)
    else:
        grad = gradient(s, ORANGE_TOP, ORANGE_BOTTOM).resize((s, s)).convert("RGBA")
        m = Image.new("L", (s, s), 0)
        ImageDraw.Draw(m).rounded_rectangle(capsule, radius=radius, fill=255)
        img.paste(grad, (0, 0), m)

    return img.resize((size, size), Image.LANCZOS)


def main():
    # 应用图标（带徽章背景）
    render(32).save(ICONS / "32x32.png")
    render(128).save(ICONS / "128x128.png")
    render(256).save(ICONS / "128x128@2x.png")

    # Windows 图标文件：多尺寸打包
    sizes = [16, 24, 32, 48, 64, 128, 256]
    render(256).save(ICONS / "icon.ico", sizes=[(n, n) for n in sizes])

    # 托盘：透明背景、只留字形、笔画加粗；active 全橙表示录音中
    render(44, glyph_only=True, stroke=1.4).save(ICONS / "tray.png")
    render(44, glyph_only=True, stroke=1.4, tint=(216, 85, 48)).save(ICONS / "tray-active.png")

    for f in sorted(ICONS.glob("*.png")) + sorted(ICONS.glob("*.ico")):
        print(f.relative_to(ROOT), f.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
