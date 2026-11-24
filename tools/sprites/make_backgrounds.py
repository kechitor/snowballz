"""The full screen backgrounds (menu, loading, winner, loser) and the maps'
pictures, rendered from the original Blender files, with a cloudy sky and
the titles drawn behind and over them.

    python3 tools/sprites/make_backgrounds.py BLENDS_DIR [BLENDER_2.79]

Like make_sprites.py (BLENDS_DIR, Blender 2.79, Pillow and numpy).
smallcrystalrush.blend and whiteforest.blend are made in BLENDS_DIR if
they are missing (make_crystalrush.py, make_whiteforest.py).  Writes
data/bg_*.jpg and the icon.png in each of maps/*.zip.

A background is 3440x1440: the old 640x480 picture is its 1920x1440 middle
(see src/backgrounds.rs), and the camera sees more of the scene round it.
A map's picture is 768x768, drawn 256 wide in the menu."""
import io, os, subprocess, sys, tempfile, zipfile
from concurrent.futures import ThreadPoolExecutor
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, '..', '..')
FONT = os.path.join(ROOT, 'data', 'TSCu_Comic.ttf')

BG_W, BG_H = 3440, 1440
K = 3                       # canvas pixels per old picture pixel
X0 = (BG_W - 640 * K) // 2  # where the old picture starts
ICON = 768

# background: (blend file, (x, y, w, h) of the file's frame in the old
# picture, render options, titles: [(text, (x, y) of the ink's top left,
# width of the ink)] in the old picture's pixels)
PRESS = 'Press any key to continue'
BACKGROUNDS = {
    'loading': ('penguinpos', (0, 0, 640, 480), [], [('Loading', (36, 32), 274)]),
    'winner': ('winner', (0, 0, 640, 480), [], [('Winner', (28, 52), 257), (PRESS, (338, 95), 228)]),
    'loser': ('loser', (0, 0, 640, 480), [], [('Loser', (74, 52), 197), (PRESS, (308, 93), 228)]),
    # the menu's picture is the loading picture made smaller (menu.blend is
    # a different, unused, scene); the penguins beyond its old right edge
    # would stand behind the logo
    'menu': ('penguinpos', (1, -71, 397, 298), ['hide=Armature.006', 'hide=Armature.008'], []),
}

# map: (blend file, render options, the sky's colours or None)
ICONS = {
    'crystalcrises': ('crystal_crises', [], {}),
    # the file's camera looks down; the old picture was taken from in front,
    # with no ground: the statue stands in white haze
    'whiteforest': ('whiteforest', [], {'bottom': (240, 244, 255), 'horizon': 1.0}),
    'iceland': ('lots_of_ice', [], None),
    'smallcrystalrush': ('smallcrystalrush', [], {}),
}


def blender_run(blender, blend, script, *args):
    subprocess.run([blender, '-b', blend, '--python', os.path.join(HERE, script), '--', *args],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def blend_path(blends, name):
    return os.path.join(blends, name if '.blend' in name else name + '.blend')


def saved_by_279(blender, blends, tmp, name):
    """A copy of a 2.4x file saved by Blender 2.79, to append from."""
    out = os.path.join(tmp, name + '.blend')
    if not os.path.exists(out):
        subprocess.run([blender, '-b', blend_path(blends, name), '--python-expr',
                        'import bpy; bpy.ops.wm.save_as_mainfile(filepath=%r)' % out],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return out


def made_blends(blender, blends, tmp):
    out = os.path.join(blends, 'smallcrystalrush.blend')
    if not os.path.exists(out):
        blender_run(blender, blend_path(blends, 'crystal'), 'make_crystalrush.py',
                    saved_by_279(blender, blends, tmp, 'worker_gathering_ice'),
                    saved_by_279(blender, blends, tmp, 'crystal_crises'), out)
    out = os.path.join(blends, 'whiteforest.blend')
    if not os.path.exists(out):
        trees = ['tree'] + ['tree%d' % i for i in range(1, 7)]
        blender_run(blender, blend_path(blends, 'snowmound3.blend1'), 'make_whiteforest.py',
                    *[saved_by_279(blender, blends, tmp, t) for t in trees], out)


def render(blender, blends, tmp, name, blend, size, frame, options):
    out = os.path.join(tmp, name + '.png')
    blender_run(blender, blend_path(blends, blend), 'render_view.py', out, *map(str, size + frame), *options)
    return Image.open(out).convert('RGBA')


def noise(w, h, rng, across, down=None):
    """Smooth random values, (about) `across` blobs across and `down` down
    (else round ones)."""
    gw, gh = across + 3, (down or max(2, round(across * h / w))) + 3
    grid = Image.fromarray(rng.random((gh, gw)).astype('float32'), 'F')
    # (the outer cells are cut off: bicubic overshoots at the edges)
    big = grid.resize((round(w * gw / (gw - 3)), round(h * gh / (gh - 3))), Image.BICUBIC)
    cx, cy = (big.width - w) // 2, (big.height - h) // 2
    return np.asarray(big.crop((cx, cy, cx + w, cy + h)))


def sky(w, h, seed, top=(130, 120, 255), bottom=(55, 140, 255), clouds=1.0, horizon=0.65):
    """Blue sky, violet up high, hazy with soft white clouds."""
    rng = np.random.default_rng(seed)
    n = sum(noise(w, h, rng, 4 * 2 ** o) * 0.7 ** o for o in range(8))
    n = (n - n.min()) / (n.max() - n.min())
    c = np.clip((n - 0.15) / 0.85, 0, 1)
    c = c * c * (3 - 2 * c) * clouds
    t = np.clip(np.arange(h) / (h * horizon), 0, 1)[:, None, None]
    base = np.array(top) * (1 - t) + np.array(bottom) * t
    rgb = base * (1 - c[..., None]) + np.array([238, 238, 255]) * c[..., None]
    return Image.fromarray(np.clip(rgb, 0, 255).astype('uint8'), 'RGB').convert('RGBA')


def font_for(text, width):
    """The title font sized so `text` is `width` canvas pixels wide."""
    f = ImageFont.truetype(FONT, 100)
    b = f.getbbox(text)
    return ImageFont.truetype(FONT, round(100 * width / (b[2] - b[0])))


def title(img, text, pos, width):
    """Red, with a soft dark shadow, like the old titles."""
    f = font_for(text, width * K)
    b = f.getbbox(text)
    x, y = X0 + pos[0] * K - b[0], pos[1] * K - b[1]
    s = max(2, f.size // 30)
    shadow = Image.new('L', img.size)
    ImageDraw.Draw(shadow).text((x + s, y + s), text, font=f, fill=180)
    shadow = shadow.filter(ImageFilter.GaussianBlur(s))
    img.alpha_composite(Image.merge('RGBA', [Image.new('L', img.size)] * 3 + [shadow]))
    ImageDraw.Draw(img).text((x, y), text, font=f, fill=(225, 3, 4, 255))


def logo(img, cx, cy):
    """The SnowballZ logo: the name on a white patch in a black ink splat."""
    rng = np.random.default_rng(7)
    k = K * 2  # drawn bigger again and shrunk, for smooth edges
    w, h = 340 * k, 240 * k
    ink = Image.new('L', (w, h))
    d = ImageDraw.Draw(ink)
    ox, oy, rx, ry = w / 2, h / 2, 118 * k, 33 * k

    def blob(a, out, r):
        ex, ey = np.cos(a) * (rx + out), np.sin(a) * (ry + out)
        d.ellipse([ox + ex - r, oy + ey - r, ox + ex + r, oy + ey + r], fill=255)

    # a ragged rim round the patch
    for _ in range(700):
        blob(rng.uniform(0, 2 * np.pi), rng.exponential(3) * k + 5 * k, rng.uniform(3, 7) * k)
    # stubby spikes: runs of blobs thinning out, longest up and down
    for _ in range(260):
        a = rng.uniform(0, 2 * np.pi)
        length = rng.exponential(4 + 12 * abs(np.sin(a))) * k
        for t in np.arange(0, length, k):
            blob(a, 6 * k + t, (4.5 - 2.5 * t / max(length, 1)) * k)
        # and a drop flung off the end
        if rng.random() < 0.3:
            blob(a, 10 * k + length * 1.3, rng.uniform(1.5, 3) * k)
    d.ellipse([ox - rx - 8 * k, oy - ry - 8 * k, ox + rx + 8 * k, oy + ry + 8 * k], fill=255)
    ink = ink.filter(ImageFilter.GaussianBlur(1.6 * k))
    patch = Image.new('L', (w, h))
    ImageDraw.Draw(patch).ellipse([ox - rx, oy - ry, ox + rx, oy + ry], fill=255)
    patch = patch.filter(ImageFilter.GaussianBlur(3 * k))
    layer = Image.merge('RGBA', [patch] * 3 + [ink])
    text = 'SnowballZ'
    f = font_for(text, 232 * k)
    b = f.getbbox(text, stroke_width=k)
    tx, ty = ox - (b[0] + b[2]) / 2, oy - (b[1] + b[3]) / 2
    d = ImageDraw.Draw(layer)
    d.text((tx, ty), text, font=f, fill=(255, 255, 255, 255), stroke_width=3 * k, stroke_fill=(255, 255, 255, 255))
    d.text((tx + k, ty + k), text, font=f, fill=(120, 0, 0, 255), stroke_width=k, stroke_fill=(120, 0, 0, 255))
    d.text((tx, ty), text, font=f, fill=(235, 10, 10, 255), stroke_width=k, stroke_fill=(235, 10, 10, 255))
    layer = layer.resize((w * K // k, h * K // k), Image.LANCZOS)
    img.alpha_composite(layer, (round(X0 + cx * K - layer.width / 2), round(cy * K - layer.height / 2)))


def wood(w, h):
    """The menu's wooden bar: grain running along it."""
    rng = np.random.default_rng(3)
    lines = sum(noise(w, h, rng, 4 * 2 ** o, 6 * 2 ** o) * 0.6 ** o for o in range(4))
    g = (np.sin(lines * 9 * np.pi) * 0.5 + 0.5)[..., None]
    rgb = np.array([150, 98, 12]) * (1 - g) + np.array([206, 152, 46]) * g
    img = Image.fromarray(np.clip(rgb, 0, 255).astype('uint8'), 'RGB').convert('RGBA')
    ImageDraw.Draw(img).rectangle([0, 0, w, K - 1], fill=(60, 38, 4, 255))
    return img


def background(blender, blends, tmp, name):
    blend, (x, y, w, h), options, titles = BACKGROUNDS[name]
    shot = render(blender, blends, tmp, 'bg-' + name, blend, [BG_W, BG_H], [X0 + x * K, y * K, w * K, h * K], options)
    if name == 'menu':
        img = sky(BG_W, BG_H, 1, top=(85, 78, 255), bottom=(185, 182, 255), clouds=0.35, horizon=0.07)
        ImageDraw.Draw(img).rectangle([0, 105 * K, BG_W, BG_H], fill=(255, 255, 255, 255))
        # the snow fades into the white below the picture, as it did
        a = np.asarray(shot).copy()
        ramp = np.clip((1000 - np.arange(BG_H)) / 400, 0, 1)[:, None]
        a[..., 3] = (a[..., 3] * ramp).astype('uint8')
        img.alpha_composite(Image.fromarray(a, 'RGBA'))
        logo(img, 496, 117)
        img.alpha_composite(wood(BG_W, 64 * K), (0, 416 * K))
    else:
        img = sky(BG_W, BG_H, list(BACKGROUNDS).index(name))
        img.alpha_composite(shot)
    for t in titles:
        title(img, *t)
    return img.convert('RGB')


def icon(blender, blends, tmp, name):
    blend, options, colours = ICONS[name]
    shot = render(blender, blends, tmp, 'icon-' + name, blend, [ICON, ICON], [0, 0, ICON, ICON], options)
    if colours is None:
        img = Image.new('RGBA', shot.size, (255, 255, 255, 255))
    else:
        img = sky(ICON, ICON, 5, **{'horizon': 0.8, **colours})
    img.alpha_composite(shot)
    return img


def put_icon(path, img):
    """Replace (or add) icon.png in a map's zip file."""
    buf = io.BytesIO()
    img.save(buf, 'PNG', optimize=True)
    with zipfile.ZipFile(path) as z:
        items = [(i, z.read(i)) for i in z.infolist() if i.filename != 'icon.png']
    tmp = path + '.tmp'
    with zipfile.ZipFile(tmp, 'w') as z:
        for i, data in items:
            z.writestr(i, data)
        z.writestr(zipfile.ZipInfo('icon.png', items[0][0].date_time), buf.getvalue(), zipfile.ZIP_DEFLATED)
    os.replace(tmp, path)


def main():
    blends = sys.argv[1]
    blender = sys.argv[2] if len(sys.argv) > 2 else 'blender'
    tmp = tempfile.mkdtemp(prefix='snowballz-backgrounds-')
    made_blends(blender, blends, tmp)
    jobs = [(background, n) for n in BACKGROUNDS] + [(icon, n) for n in ICONS]
    with ThreadPoolExecutor(os.cpu_count()) as pool:
        for (make, name), img in zip(jobs, pool.map(lambda j: j[0](blender, blends, tmp, j[1]), jobs)):
            if make is background:
                img.save(os.path.join(ROOT, 'data', 'bg_%s.jpg' % name), quality=92, subsampling=0)
            else:
                put_icon(os.path.join(ROOT, 'maps', name + '.zip'), img)
            print(name, img.size)


if __name__ == '__main__':
    main()
