"""The game's icon: the frozen worker of workerfrozen.blend in its ice block,
on a rounded square of midnight sky.  Writes icon.png (the window's icon)
and the packages' icons, tools/release/snowballz-512.png and
snowballz.icns (macOS, up to 1024x1024).

    python3 tools/sprites/make_icon.py BLENDS_DIR [BLENDER_2.79]

Like make_sprites.py (BLENDS_DIR, Blender 2.79, Pillow and numpy)."""
import os, random, subprocess, sys, tempfile
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, '..', '..')
N = 1024     # drawn at twice the size, for smooth edges
SIZE = 512


def sky():
    y = np.linspace(0, 1, N)[:, None]
    g = (np.array([34, 44, 120]) * (1 - y) + np.array([10, 14, 48]) * y)[:, None, :].repeat(N, 1)
    # a glow behind the ice block
    yy, xx = np.mgrid[0:N, 0:N]
    d = np.hypot(xx - N * 0.5, yy - N * 0.47) / (N * 0.45)
    g = g + np.clip(1 - d, 0, 1)[..., None] ** 2 * np.array([40, 50, 110])
    img = Image.fromarray(np.clip(g, 0, 255).astype('uint8'), 'RGB').convert('RGBA')
    stars = Image.new('RGBA', (N, N))
    draw, rnd = ImageDraw.Draw(stars), random.Random(7)
    for i in range(45):
        x, y, s = rnd.uniform(40, N - 40), rnd.uniform(40, N * 0.55), rnd.uniform(2, 5)
        draw.ellipse((x - s, y - s, x + s, y + s), fill=(235, 240, 255, int(rnd.uniform(90, 220))))
    img.alpha_composite(stars.filter(ImageFilter.GaussianBlur(1.2)))
    return img


def main():
    blends = sys.argv[1]
    blender = sys.argv[2] if len(sys.argv) > 2 else 'blender'
    shot = os.path.join(tempfile.mkdtemp(prefix='snowballz-icon-'), 'frozen.png')
    subprocess.run([blender, '-b', os.path.join(blends, 'workerfrozen.blend'), '--python',
                    os.path.join(HERE, 'render_view.py'), '--', shot, '1024', '1024', '0', '0', '1024', '1024', 'frame=1'],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    penguin = Image.open(shot).convert('RGBA')
    # the ice block in the middle, 700 high (its shadow left out of the measure)
    a = np.asarray(penguin).astype(int)
    ys, xs = np.nonzero((a[..., 3] > 200) & (a[..., :3].max(-1) > 70))
    cx, cy = (xs.min() + xs.max()) / 2, (ys.min() + ys.max()) / 2
    k = 700 / max(xs.max() - xs.min(), ys.max() - ys.min())
    penguin = penguin.resize((round(penguin.width * k), round(penguin.height * k)), Image.LANCZOS)
    img = sky()
    img.alpha_composite(penguin, (round(N / 2 - cx * k), round(N / 2 - cy * k)))
    mask = Image.new('L', (N, N))
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, N - 1, N - 1), radius=190, fill=255)
    out = Image.new('RGBA', (N, N))
    out.paste(img, (0, 0), mask)
    small = out.resize((SIZE, SIZE), Image.LANCZOS)
    small.save(os.path.join(ROOT, 'icon.png'), optimize=True)
    small.save(os.path.join(ROOT, 'tools', 'release', 'snowballz-512.png'), optimize=True)
    out.save(os.path.join(ROOT, 'tools', 'release', 'snowballz.icns'))


if __name__ == '__main__':
    main()
