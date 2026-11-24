"""The pictures of the menus and the game's panels, at 4 times their old
size (the game draws them at the old size): the theme's buttons, text
fields and panel background (data/themes/snowballz/), the selection
panel's ice blocks, the mouse cursor, the easel under a map's picture, the
"no icon" picture and the lobby's igloo (rendered from
igloo_construction.blend).  Same shapes, places and colours as the old
pictures.

    python3 tools/sprites/make_ui.py BLENDS_DIR [BLENDER_2.79]

Like make_sprites.py (BLENDS_DIR, Blender 2.79, Pillow and numpy)."""
import math, os, sys, tempfile
import numpy as np
from PIL import Image, ImageDraw, ImageFont
from make_effects import Canvas, K, inside
from make_backgrounds import FONT, ROOT, noise, render

THEME = os.path.join(ROOT, 'data', 'themes', 'snowballz')


def rounded_box(c, x0, y0, x1, y1, r):
    """Signed distance to a rounded rectangle."""
    qx = np.abs(c.x - (x0 + x1) / 2) - ((x1 - x0) / 2 - r)
    qy = np.abs(c.y - (y0 + y1) / 2) - ((y1 - y0) / 2 - r)
    return np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r


def ramp(t, stops):
    """Colour at t (0..1) between evenly spaced colour stops."""
    stops = np.array(stops, float)
    p = np.clip(t, 0, 1) * (len(stops) - 1)
    i = np.minimum(p.astype(int), len(stops) - 2)
    f = (p - i)[..., None]
    return stops[i] * (1 - f) + stops[i + 1] * f


# the buttons: (border, gradient top / middle / bottom), as the old ones
BUTTONS = {
    'button': ((81, 120, 230), [(95, 103, 209), (69, 96, 217), (116, 167, 250)]),
    'button_focused': ((153, 176, 240), [(95, 103, 209), (69, 96, 217), (116, 167, 250)]),
    'button_hover': ((153, 176, 240), [(113, 125, 218), (95, 121, 224), (127, 169, 246)]),
    'button_down': ((153, 176, 240), [(68, 87, 199), (51, 83, 205), (83, 131, 227)]),
}


def button(border, stops, pressed):
    """20 x 30 (9-sliced by the game: 6 pixels at the sides, 8 at the top
    and bottom stay as they are)."""
    c = Canvas(20, 30)
    outer = rounded_box(c, 0, 0, 20, 30, 3.5)
    c.paint(inside(outer, 0.4), border)
    body = rounded_box(c, 1, 1, 19, 29, 2.6)
    c.paint(inside(body, 0.4), ramp((c.y - 1) / 28, stops))
    if pressed:
        # pressed in: shadow along the top
        c.paint(inside(body, 0.4) * np.clip(1 - (c.y - 1) / 5, 0, 1), (20, 30, 90), 0.35)
    else:
        # a soft gloss on the upper half
        gloss = inside(rounded_box(c, 2, 1.6, 18, 13, 2.2), 0.8)
        c.paint(gloss * np.clip(1 - (c.y - 1.6) / 11.4, 0, 1), (255, 255, 255), 0.12)
    return c


def text_field(border):
    """A text field: a button's frame round a sunken darker field."""
    c = Canvas(20, 30)
    c.paint(inside(rounded_box(c, 0, 0, 20, 30, 3.5), 0.4), border)
    c.paint(inside(rounded_box(c, 1, 1, 19, 29, 2.6), 0.4), ramp((c.y - 1) / 28, [(57, 87, 214), (57, 87, 214), (116, 167, 250)]))
    field = rounded_box(c, 2.5, 2.5, 17.5, 27.5, 1.8)
    c.paint(inside(field, 0.4), ramp((c.y - 2.5) / 25, [(68, 80, 158), (75, 94, 172), (91, 121, 187)]))
    # sunken: shadow inside its top and left edges
    shadow = inside(field, 0.4) * np.clip(1 - np.minimum(c.x - 2.5, c.y - 2.5) / 2.2, 0, 1)
    c.paint(shadow, (25, 35, 95), 0.45)
    return c


def theme():
    for name, (border, stops) in BUTTONS.items():
        button(border, stops, name == 'button_down').image().save(os.path.join(THEME, name + '.png'), optimize=True)
    text_field((81, 120, 230)).image().save(os.path.join(THEME, 'input.png'), optimize=True)
    text_field((153, 176, 240)).image().save(os.path.join(THEME, 'input_focused.png'), optimize=True)
    # the panels' background, tiled: warm orange with fine seamless grain
    n = 16 * K
    f = np.hypot(*np.meshgrid(np.fft.fftfreq(n) * n, np.fft.fftfreq(n) * n))
    rng = np.random.default_rng(4)
    g = np.real(np.fft.ifft2(np.fft.fft2(rng.standard_normal((n, n))) * ((f > 2) & (f < 24)) / np.maximum(f, 1) ** 0.5))
    g = g / np.abs(g).max()
    rgb = np.array([200.2, 128.2, 41.1]) + g[..., None] * np.array([11, 9, 7])
    Image.fromarray(np.clip(rgb + 0.5, 0, 255).astype('uint8'), 'RGB').save(os.path.join(THEME, 'bg.png'), optimize=True)


def select_panel():
    """The selection panel's stack of ice blocks (64 x 256, the blocks in
    its lower 160: 64, 64 and 32 high), see of data/ice.png."""
    c = Canvas(64, 256, 2)
    ice = np.asarray(Image.open(os.path.join(ROOT, 'data', 'ice.png')).convert('RGB')).astype(float)
    tex = np.tile(ice, (c.a.shape[0] // ice.shape[0] + 1, c.a.shape[1] // ice.shape[1] + 1, 1))[:c.a.shape[0], :c.a.shape[1]]
    for y0, y1 in ((96, 160), (160, 224), (224, 256)):
        block = rounded_box(c, 0.3, y0 + 0.3, 63.7, y1 - 0.3, 2.5)
        c.paint(inside(block, 0.5), tex * 0.35 + np.array([214, 214, 250]) * 0.65, 0.86)
        # bright bevel along the top and left, darker along the bottom and right
        rim = inside(block, 0.5) * inside(-block - 2.2, 1.2)
        light = np.clip(((c.x - 32) * -0.5 + (c.y - (y0 + y1) / 2) * -1.0) / 30 + 0.5, 0, 1)
        c.paint(rim, ramp(light, [(150, 150, 215), (236, 236, 255)]), 0.9)
        # a diagonal gloss streak
        d = (c.x + (c.y - y0)) - (y1 - y0) * 0.75
        c.paint(inside(block + 3, 0.5) * np.exp(-(d / 7) ** 2), (255, 255, 255), 0.28)
    return c


def cursor():
    """An icicle, its point (the hot spot) at the top left."""
    c = Canvas(64, 64)
    ax, ay = np.array([52.0, 49.0]) / np.hypot(52, 49)
    t = c.x * ax + c.y * ay                       # along the icicle
    s = -c.x * ay + c.y * ax                      # across it
    length = 64.5
    radius = 0.6 + 8.4 * np.clip(t / length, 0, 1) ** 0.9
    body = np.maximum(np.abs(s) - radius, t - length)
    cover = inside(np.maximum(body, -t), 0.6)
    across = np.clip(s / np.maximum(radius, 0.1), -1, 1)
    light = np.clip(0.55 - 0.6 * across, 0, 1)      # lit from the upper right
    streak = 0.5 + 0.5 * np.sin(t * 0.9 + 3 * np.sin(t * 0.21)) * (1 - np.abs(across))
    v = 168 + 80 * light + 8 * streak
    c.paint(cover * 0.25, (40, 60, 120), 1)
    c.paint(cover, np.stack([v - 6, v - 2, np.minimum(v + 12, 255)], -1), 0.95)
    # the broken-off end: an oval of fresher ice
    ex, ey = 52.0 * length / np.hypot(52, 49), 49.0 * length / np.hypot(52, 49)
    end = np.hypot((c.x - ex) * ax + (c.y - ey) * ay, (-(c.x - ex) * ay + (c.y - ey) * ax) / 2.6) - 3.3
    c.paint(inside(end, 0.5), (205, 215, 240))
    c.paint(inside(end + 1.2, 0.5), (238, 242, 255))
    # glint
    c.paint(inside(np.hypot(s + radius * 0.45, (t - length * 0.55) / 9) - 0.7, 0.6) * (t > 5), (255, 255, 255), 0.7)
    return c


def easel():
    """The board under a map's picture (266 square, its frame 5 wide) on
    two legs."""
    w, h = 266, 291
    c = Canvas(w, h, 1)     # (big: its soft edges are smooth enough)
    rng = np.random.default_rng(6)
    hh, ww = c.a.shape
    grain_l = sum(noise(ww, hh, rng, 2 * 2 ** o, 30 * 2 ** o) * 0.6 ** o for o in range(4))
    grain = (np.sin(grain_l * 10 * np.pi) * 0.5 + 0.5)
    fine = noise(ww, hh, rng, 6, 260)
    wood = ramp(grain * 0.85 + fine * 0.15, [(150, 98, 12), (180, 122, 22), (206, 152, 46)])
    # frame: darker wood, lit at the top left
    board = rounded_box(c, 0, 0, 266, 266, 3)
    c.paint(inside(board, 0.4), wood * 0.78)
    frame_light = np.clip(1 - np.minimum(c.x, c.y) / 5, 0, 1) - np.clip(1 - np.minimum(266 - c.x, 266 - c.y) / 5, 0, 1)
    c.paint(inside(board, 0.4) * np.clip(frame_light, 0, 1), (255, 220, 150), 0.18)
    c.paint(inside(board, 0.4) * np.clip(-frame_light, 0, 1), (40, 20, 0), 0.25)
    # the panel inside, a step down
    inner = rounded_box(c, 5, 5, 261, 261, 1)
    c.paint(inside(inner, 0.4), wood)
    c.paint(inside(inner, 0.4) * np.clip(1 - np.minimum(c.x - 5, c.y - 5) / 3, 0, 1), (60, 35, 0), 0.45)
    # legs
    for x0 in (36, 210):
        leg = rounded_box(c, x0, 262, x0 + 21, 291, 1.5)
        shade = np.clip((c.x - x0) / 21, 0, 1)
        c.paint(inside(leg, 0.4), wood * (0.92 - 0.22 * shade[..., None]))
        c.paint(inside(leg, 0.4) * np.clip(1 - (c.y - 266) / 6, 0, 1) * (c.y > 266), (40, 20, 0), 0.5)
    return c


def no_icon():
    """The picture of a map that has none."""
    s = 256 * K
    img = Image.new('RGBA', (s * 2, s * 2))
    d = ImageDraw.Draw(img)
    k = 2 * K
    for a, b in (((8, 8), (88, 88)), ((248, 8), (168, 88)), ((8, 248), (88, 168)), ((248, 248), (168, 168))):
        d.line([(a[0] * k, a[1] * k), (b[0] * k, b[1] * k)], fill=(60, 60, 70, 210), width=7 * k)
    f = ImageFont.truetype(FONT, 34 * k)
    d.text((128 * k, 128 * k), 'No Icon', font=f, fill=(60, 60, 70, 230), anchor='mm')
    return img.resize((s, s), Image.LANCZOS)


def main():
    blends = sys.argv[1]
    blender = sys.argv[2] if len(sys.argv) > 2 else 'blender'
    theme()
    data = os.path.join(ROOT, 'data')
    select_panel().image().save(os.path.join(data, 'selectctrl.png'), optimize=True)
    cursor().image().save(os.path.join(data, 'mouse_cursor.png'), optimize=True)
    easel().image().save(os.path.join(data, 'map_img_panel.png'), optimize=True)
    no_icon().save(os.path.join(data, 'noicon.png'), optimize=True)
    tmp = tempfile.mkdtemp(prefix='snowballz-ui-')
    s = 256 * K
    render(blender, blends, tmp, 'lobby', 'igloo_construction', [s, s], [0, 0, s, s], []).save(os.path.join(data, 'lobbybg.png'), optimize=True)


if __name__ == '__main__':
    main()
