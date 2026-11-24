"""The small pictures of the game's effects, at 4 times their old size (the
game draws them at the old size, see texture_sized in src/gfx.rs): the
snowball and its shadow, its imprints in snow and on ice, footprints, the
igloos' smoke, snow particles and flakes, the selection box and the flag of
a frozen penguin.  Same places and colours as the old pictures; white
parts are tinted by the game where the old ones were.

    python3 tools/sprites/make_effects.py

Needs Pillow and numpy."""
import math, os
import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, '..', '..')
K = 4    # file pixels per game pixel
SS = 4   # drawn SS times bigger again, then shrunk: smooth edges


class Canvas:
    """A w x h (game pixels) picture; x, y in game pixels."""

    def __init__(self, w, h, ss=SS):
        self.w, self.h, self.ss = w, h, ss
        n = K * ss
        ys, xs = np.mgrid[0:h * n, 0:w * n]
        self.x, self.y = (xs + 0.5) / n, (ys + 0.5) / n
        self.rgb = np.zeros((h * n, w * n, 3))
        self.a = np.zeros((h * n, w * n))

    def paint(self, cover, rgb, alpha=1.0):
        """Paint colour `rgb` (0-255, or an array of them) over the picture
        where `cover` (0..1) says, at opacity `alpha`."""
        c = np.clip(cover, 0, 1) * alpha
        rgb = np.broadcast_to(np.asarray(rgb, float), self.rgb.shape)
        a = self.a + c * (1 - self.a)
        # (over, with premultiplied colours)
        pre = self.rgb * (self.a * (1 - c))[..., None] + rgb * c[..., None]
        self.rgb = pre / np.maximum(a, 1e-9)[..., None]
        self.a = a

    def image(self):
        # shrink with premultiplied alpha (no dark fringes)
        n = self.ss
        h, w = self.a.shape[0] // n, self.a.shape[1] // n
        pre = np.concatenate([self.rgb * self.a[..., None], self.a[..., None] * 255], -1)
        small = pre.reshape(h, n, w, n, 4).mean((1, 3))
        a = small[..., 3:] / 255
        rgb = np.where(a > 0, small[..., :3] / np.maximum(a, 1e-6), 0)
        out = np.concatenate([rgb, a * 255], -1)
        return Image.fromarray(np.clip(out + 0.5, 0, 255).astype('uint8'), 'RGBA')


def inside(d, soft=0.0):
    """Coverage from a signed distance (negative inside), with an optional
    soft edge `soft` game pixels wide."""
    if soft:
        return np.clip(0.5 - d / soft, 0, 1)
    return (d < 0).astype(float)


def ball(c, cx, cy, r, light=(-0.5, -0.6)):
    """A lit snowy ball: (unit normal, coverage)."""
    dx, dy = (c.x - cx) / r, (c.y - cy) / r
    d2 = dx * dx + dy * dy
    nz = np.sqrt(np.clip(1 - d2, 0, 1))
    lx, ly = light
    lz = math.sqrt(max(0.0, 1 - lx * lx - ly * ly))
    shade = np.clip(dx * lx + dy * ly + nz * lz, 0, 1)
    return shade, inside(np.sqrt(d2) - 1)


def snowball():
    c = Canvas(16, 8)
    # the ball (5 across at 0, 0) and its shadow (5 across at 8, 0)
    shade, cover = ball(c, 2.5, 2.5, 2.45)
    rgb = np.stack([150 + 105 * shade, 158 + 97 * shade, 178 + 77 * shade], -1)
    c.paint(cover, rgb)
    d = np.hypot((c.x - 10.5) / 2.6, (c.y - 2.5) / 2.3) - 1
    c.paint(inside(d, 0.9), (0, 0, 0), 0.16)
    return c


def snowball_imprint():
    """A snowball's dent in the snow: a ring, dark where the light can't
    reach (top left), bright on the far rim, and a few crumbs."""
    c = Canvas(8, 8)
    r = np.hypot(c.x - 4, c.y - 4)
    ang = np.arctan2(c.y - 4, c.x - 4)
    light = np.cos(ang - math.radians(45))      # 1 towards the bottom right
    rim = inside(np.abs(r - 3.0) - 0.75, 0.5)
    v = 110 + 120 * (light + 1) / 2
    c.paint(rim * (0.45 + 0.55 * np.abs(light)), np.stack([v, v, v + 10], -1), 0.37)
    hollow = inside(r - 2.3, 0.6)
    c.paint(hollow * np.clip(-np.cos(ang - math.radians(45)) * 0.6 + 0.2, 0, 1), (70, 74, 100), 0.12)
    rng = np.random.default_rng(3)
    for i in range(5):
        a, d = rng.uniform(0, 2 * math.pi), rng.uniform(3.3, 3.9)
        cx, cy = 4 + d * math.cos(a), 4 + d * math.sin(a)
        c.paint(inside(np.hypot(c.x - cx, c.y - cy) - 0.35, 0.3), (235, 238, 245), 0.4)
    return c


def snowball_ice_imprint():
    """A snowball smashed on ice: a white splat with a few drops."""
    c = Canvas(8, 8)
    ang = np.arctan2(c.y - 4, c.x - 4)
    r = np.hypot(c.x - 4, c.y - 4)
    edge = 2.4 + 0.45 * np.sin(5 * ang + 1) + 0.3 * np.sin(9 * ang + 2)
    cover = inside(r - edge, 0.4)
    shade = np.clip(1 - (c.x - 4 + c.y - 4) / 6, 0, 1)
    v = 170 + 80 * shade
    c.paint(cover, np.stack([v, v, v + 5], -1))
    rng = np.random.default_rng(5)
    for i in range(6):
        a, d, s = rng.uniform(0, 2 * math.pi), rng.uniform(3.0, 3.7), rng.uniform(0.25, 0.45)
        c.paint(inside(np.hypot(c.x - 4 - d * math.cos(a), c.y - 4 - d * math.sin(a)) - s, 0.3), (232, 234, 240))
    return c


def footprint():
    """A penguin's foot pressed in the snow: a flat paddle with rounded
    corners, a little wider at the front (as on the penguin model), its
    front pointing up (the game turns it the way the penguin walks).
    Deepest under the front and the heel, where the foot presses."""
    c = Canvas(8, 8)
    top, bottom, r = 0.9, 7.1, 1.0
    # half width: 1.75 at the front, 1.25 at the heel
    t = np.clip((c.y - top) / (bottom - top), 0, 1)
    half = 1.75 - 0.5 * t
    qx = np.abs(c.x - 4) - (half - r)
    qy = np.abs(c.y - (top + bottom) / 2) - ((bottom - top) / 2 - r)
    d = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r
    dent = inside(d, 0.35)
    deep = np.maximum(np.clip(1 - (c.y - top) / 2.2, 0, 1), np.clip(1 - (bottom - c.y) / 1.8, 0, 1))
    c.paint(dent, (150, 154, 178), 0.38)
    c.paint(dent * deep, (110, 115, 145), 0.25)
    # snow pushed up round the front catches the light
    rim = inside(np.abs(d - 0.45) - 0.22, 0.3) * np.clip(1 - (c.y - top) / 2.5, 0, 1)
    c.paint(rim, (252, 253, 255), 0.35)
    return c


def smoke():
    """A puff: soft and a little lumpy (scaled up as it rises)."""
    c = Canvas(8, 8)
    rng = np.random.default_rng(9)
    puff = np.zeros_like(c.a)
    for i in range(6):
        cx, cy, s = 4 + rng.normal(0, 0.55), 4 + rng.normal(0, 0.55), rng.uniform(1.3, 1.8)
        puff = puff + np.exp(-((c.x - cx) ** 2 + (c.y - cy) ** 2) / (2 * s * s))
    puff = np.clip(puff / 2.2, 0, 1) * np.clip((4 - np.hypot(c.x - 4, c.y - 4)) / 1.2, 0, 1)
    # lit from above: lighter on top
    v = 150 + 22 * np.clip(1 - (c.y - 1.5) / 6, 0, 1)
    c.paint(puff, np.stack([v, v, v + 3], -1))
    return c


def snowparticle():
    c = Canvas(4, 4)
    shade, cover = ball(c, 2, 2, 1.8)
    c.paint(cover, np.stack([200 + 55 * shade] * 2 + [215 + 40 * shade], -1))
    return c


def flake(c, cx, cy, size, branches, turn):
    """A six-armed snowflake: arms with side branches, white with a soft
    bluish glow."""
    x, y = c.x - cx, c.y - cy
    d = np.full_like(c.a, 9.0)
    for k in range(6):
        a = turn + k * math.pi / 3
        ux, uy = math.cos(a), math.sin(a)
        t = np.clip(x * ux + y * uy, 0, size)
        d = np.minimum(d, np.hypot(x - t * ux, y - t * uy))
        for at, length in branches:
            bx, by = at * size * ux, at * size * uy
            for side in (-1, 1):
                b = a + side * math.pi / 3
                vx, vy = math.cos(b), math.sin(b)
                s = np.clip((x - bx) * vx + (y - by) * vy, 0, length * size)
                d = np.minimum(d, np.hypot(x - bx - s * vx, y - by - s * vy))
    c.paint(inside(d - 0.75, 0.9), (190, 205, 255), 0.5)
    c.paint(inside(d - 0.28, 0.25), (250, 252, 255))


def snowparticles():
    c = Canvas(16, 16)
    flake(c, 4, 4, 3.4, [(0.55, 0.4)], 0.1)
    flake(c, 12, 4, 3.3, [(0.45, 0.35), (0.75, 0.22)], 0.5)
    flake(c, 4, 12, 3.4, [(0.6, 0.45)], 0.9)
    flake(c, 12, 12, 3.2, [(0.35, 0.3), (0.7, 0.3)], 0.3)
    return c


def selected():
    """The box round a selected penguin."""
    c = Canvas(32, 32)
    x0, y0, x1, y1, r = 3.5, 11.5, 28.5, 28.5, 3
    qx = np.abs(c.x - (x0 + x1) / 2) - ((x1 - x0) / 2 - r)
    qy = np.abs(c.y - (y0 + y1) / 2) - ((y1 - y0) / 2 - r)
    box = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r
    c.paint(inside(np.abs(box) - 0.9, 0.9), (0, 0, 0), 0.12)
    c.paint(inside(np.abs(box) - 0.5, 0.4), (10, 14, 30), 0.38)
    return c


def frozen_flag():
    """The little flag over a frozen penguin, its cloth white for the game
    to tint: a shaded pole with a knob and a waving pennant."""
    c = Canvas(32, 32)
    # pole
    t = (c.x - 5) / 2
    pole = inside(np.abs(c.x - 6) - 1, 0.3) * inside(np.abs(c.y - 16.5) - 13.5, 0.3)
    v = 60 + 70 * np.clip(np.sin(np.clip(t, 0, 1) * math.pi * 0.85 + 0.3), 0, 1)
    c.paint(pole, np.stack([v, v, v + 6], -1))
    # knob
    shade, cover = ball(c, 6, 2.6, 1.4)
    c.paint(cover, np.stack([110 + 120 * shade, 110 + 120 * shade, 100 + 110 * shade], -1))
    # pennant: from the pole to a point at x 27, waving
    u = np.clip((c.x - 7) / 20, 0, 1)
    wave = 0.9 * np.sin(u * math.pi * 1.6) * u
    mid = 9.5 + wave
    half = 6.5 * (1 - u) + 0.3
    cloth = inside(np.abs(c.y - mid) - half, 0.5) * inside(7 - c.x, 0.3) * inside(c.x - 27.5, 0.6)
    fold = 0.86 + 0.14 * np.cos(u * math.pi * 3.2 + 0.4)
    edge = np.clip((np.abs(c.y - mid) - half + 1.0) / 1.0, 0, 1)
    v = 255 * fold * (1 - 0.25 * edge)
    c.paint(cloth, np.stack([v, v, v], -1))
    return c


PICTURES = {
    'snowball': snowball,
    'snowballimprent': snowball_imprint,
    'snowball_ice_imprent': snowball_ice_imprint,
    'footprint': footprint,
    'smoke': smoke,
    'snowparticle': snowparticle,
    'snowparticles': snowparticles,
    'selected': selected,
    'frozen_flag': frozen_flag,
}


def main():
    for name, make in PICTURES.items():
        make().image().save(os.path.join(ROOT, 'data', name + '.png'), optimize=True)
        print(name)


if __name__ == '__main__':
    main()
