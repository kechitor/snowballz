"""The power-up pictures (plugins/powerup/), at 4 times their old size
(the game draws them at 32 x 32): the orb of the drop countdown, and the
three power-ups - double snowball (a red tile with two snowballs), defense
(a blue shield with a cross) and range (a golden target).  Same shapes,
places and colours as the old pictures, as glossy badges.

    python3 tools/sprites/make_powerups.py

Needs Pillow and numpy."""
import math, os
import numpy as np
from make_effects import Canvas, inside, ball, ROOT

OUT = os.path.join(ROOT, 'plugins', 'powerup')


def badge(c, d, top, bottom, edge, light_y0, light_y1):
    """Fill shape `d` (signed distance) like a glossy badge: a soft shadow,
    a dark edge, a top-to-bottom gradient, a light rim and a gloss on its
    upper half."""
    # shadow, down and to the right
    c.paint(inside(shifted(c, d, 0.8, 1.2), 1.6), (10, 20, 50), 0.3)
    c.paint(inside(d, 0.45), edge)
    body = d + 0.9
    t = np.clip((c.y - light_y0) / (light_y1 - light_y0), 0, 1)[..., None]
    c.paint(inside(body, 0.45), np.array(top) * (1 - t) + np.array(bottom) * t)
    # a lighter band just inside the edge, brightest at the top
    rim = inside(body, 0.45) * inside(-body - 1.1, 0.8)
    c.paint(rim * np.clip(1.2 - t[..., 0] * 1.4, 0, 1), (255, 255, 255), 0.3)
    return body


def shifted(c, d, dx, dy):
    """`d` moved by dx, dy game pixels (the canvas's own pixels)."""
    n = round(c.ss * 4)
    sx, sy = round(dx * n), round(dy * n)
    out = np.full_like(d, d.max())
    h, w = d.shape
    out[sy:, sx:] = d[:h - sy, :w - sx]
    return out


def gloss(c, body, cx, cy, rx, ry, alpha=0.38):
    """A soft white ellipse of light, clipped to the shape."""
    g = np.clip(1 - np.hypot((c.x - cx) / rx, (c.y - cy) / ry), 0, 1) ** 0.7
    c.paint(inside(body + 0.6, 0.6) * g, (255, 255, 255), alpha)


def orb():
    """The countdown's orb (18 across, centred at 15, 16)."""
    c = Canvas(32, 32)
    d = np.hypot(c.x - 15, c.y - 16) - 9
    c.paint(inside(shifted(c, d, 0.7, 1.1), 1.6), (10, 20, 50), 0.3)
    shade, cover = ball(c, 15, 16, 9, light=(-0.45, -0.55))
    rgb = np.stack([20 + 70 * shade, 80 + 95 * shade, 170 + 85 * shade], -1)
    c.paint(cover, rgb)
    c.paint(inside(np.abs(d + 0.35) - 0.35, 0.5), (18, 50, 120), 0.8)
    gloss(c, d, 12.2, 12.0, 4.8, 3.0, 0.55)
    return c


def double_snowball():
    """A red tile on its corner with two snowballs."""
    c = Canvas(32, 32)
    # a rounded square turned 45 degrees: centre 16, 16
    u = ((c.x - 16) + (c.y - 16)) / math.sqrt(2)
    v = (-(c.x - 16) + (c.y - 16)) / math.sqrt(2)
    half, r = 8.4, 2.2
    qx, qy = np.abs(u) - (half - r), np.abs(v) - (half - r)
    d = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r
    body = badge(c, d, (240, 70, 60), (175, 20, 25), (110, 10, 15), 5, 27)
    gloss(c, body, 13, 10, 7, 4.5, 0.3)
    for bx, by in ((13.2, 13.0), (18.8, 19.0)):
        c.paint(inside(np.hypot(c.x - bx - 0.5, c.y - by - 0.8) - 3.4, 1.0), (70, 0, 0), 0.35)
        shade, cover = ball(c, bx, by, 3.4)
        c.paint(cover, np.stack([185 + 70 * shade, 190 + 65 * shade, 205 + 50 * shade], -1))
    return c


def defense():
    """A blue shield with a white cross."""
    c = Canvas(32, 32)
    x, y = c.x - 15.5, c.y
    # flat top with rounded corners, sides curving into a point at the bottom
    hw = np.where(y < 14, 10.0, 10.0 * np.sqrt(np.clip(1 - ((y - 14) / 13.6) ** 2, 0, 1)))
    side = np.abs(x) - hw
    d = np.maximum(np.maximum(side, 5.0 - y), y - 27.6)
    # round the top corners
    corner = np.hypot(np.abs(x) - 8.0, y - 7.0) - 2.0
    d = np.where((np.abs(x) > 8.0) & (y < 7.0), np.maximum(corner, d), d)
    body = badge(c, d, (90, 160, 245), (30, 95, 200), (20, 55, 130), 5, 27)
    gloss(c, body, 12.0, 9.5, 7, 4, 0.28)
    # the cross, with a soft shadow
    cross = np.minimum(np.maximum(np.abs(x) - 1.7, np.abs(y - 14.5) - 6.0),
                       np.maximum(np.abs(x) - 6.6, np.abs(y - 14.5) - 1.7))
    c.paint(inside(shifted(c, cross, 0.5, 0.7), 0.9), (10, 40, 110), 0.4)
    c.paint(inside(cross, 0.4), (248, 250, 255))
    c.paint(inside(cross, 0.4) * np.clip((c.y - 9) / 12, 0, 1), (190, 205, 235), 0.6)
    return c


def target():
    """A golden target: rings, a bullseye and four sight marks."""
    c = Canvas(32, 32)
    cx, cy = 16, 16
    r = np.hypot(c.x - cx, c.y - cy)
    d = r - 10.2
    body = badge(c, d, (250, 200, 60), (200, 135, 10), (125, 80, 0), 6, 26)
    # white rings and bullseye
    c.paint(inside(np.abs(r - 5.6) - 0.85, 0.45) * inside(body, 0.4), (255, 250, 235))
    c.paint(inside(r - 2.0, 0.45), (255, 250, 235))
    c.paint(inside(r - 1.0, 0.45), (215, 140, 10))
    # sight marks at top, bottom, left and right
    marks = np.minimum(np.maximum(np.abs(c.x - cx) - 0.75, np.abs(np.abs(c.y - cy) - 8.2) - 1.6),
                       np.maximum(np.abs(c.y - cy) - 0.75, np.abs(np.abs(c.x - cx) - 8.2) - 1.6))
    c.paint(inside(marks, 0.45), (255, 250, 235))
    gloss(c, body, 13, 10.5, 6.5, 4, 0.3)
    return c


PICTURES = {
    'powerup': orb,
    'powerup_1': double_snowball,
    'powerup_2': defense,
    'powerup_3': target,
}


def main():
    for name, make in PICTURES.items():
        make().image().save(os.path.join(OUT, name + '.png'), optimize=True)
        print(name)


if __name__ == '__main__':
    main()
