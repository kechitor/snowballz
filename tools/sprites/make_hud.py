"""The pieces the in-game HUD is drawn with, at 4 times their size (the
game draws them at their size, see slice9 in src/gfx.rs), white so the game
can colour them (data/hud/):

* round.png: a rounded box (corners 12 across), stretched into panels,
  buttons and bars;
* edge.png: its 1 pixel outline;
* shadow.png: the soft shadow under a panel (16 wider all round);
* icons.png: fish, crystal, penguin and warning icons, 24 across each.

    python3 tools/sprites/make_hud.py

Needs Pillow and numpy."""
import math, os
import numpy as np
from make_effects import Canvas, inside, ROOT

OUT = os.path.join(ROOT, 'data', 'hud')
WHITE = (255, 255, 255)


def rounded_box(c, x0, y0, x1, y1, r):
    """Signed distance to a rounded rectangle."""
    qx = np.abs(c.x - (x0 + x1) / 2) - ((x1 - x0) / 2 - r)
    qy = np.abs(c.y - (y0 + y1) / 2) - ((y1 - y0) / 2 - r)
    return np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r


def segment(c, ax, ay, bx, by):
    vx, vy = bx - ax, by - ay
    t = np.clip(((c.x - ax) * vx + (c.y - ay) * vy) / (vx * vx + vy * vy), 0, 1)
    return np.hypot(c.x - ax - t * vx, c.y - ay - t * vy)


def round_box():
    c = Canvas(32, 32)
    c.paint(inside(rounded_box(c, 0, 0, 32, 32, 12), 0.6), WHITE)
    return c


def edge():
    c = Canvas(32, 32)
    d = rounded_box(c, 0, 0, 32, 32, 12)
    c.paint(inside(np.abs(d + 0.5) - 0.5, 0.5), WHITE)
    return c


def shadow():
    c = Canvas(64, 64, 1)
    d = rounded_box(c, 16, 16, 48, 48, 12)
    c.paint(np.exp(-(np.maximum(d + 2, 0) / 6.5) ** 2), (0, 0, 0))
    return c


def icons():
    c = Canvas(96, 24)
    line = lambda d, w=1.0: inside(d - w, 0.6)
    # fish: an outlined body, a tail and an eye
    x0 = 0
    body = np.hypot((c.x - x0 - 10.5) / 7.0, (c.y - 12) / 4.6) - 1
    tail = np.minimum(segment(c, x0 + 17, 12, x0 + 22, 7.5), segment(c, x0 + 17, 12, x0 + 22, 16.5))
    tail = np.minimum(tail, segment(c, x0 + 22, 7.5, x0 + 22, 16.5))
    c.paint(line(np.abs(body * 4.6), 1.0) * (c.x < x0 + 24), WHITE)
    c.paint(line(tail, 1.0), WHITE)
    c.paint(inside(np.hypot(c.x - x0 - 7, c.y - 11) - 1.2, 0.6), WHITE)
    # crystal: a gem outline with its girdle
    x0 = 24
    pts = [(x0 + 12, 2.5), (x0 + 19.5, 9), (x0 + 12, 21.5), (x0 + 4.5, 9)]
    d = np.full_like(c.a, 99.0)
    for i in range(4):
        (ax, ay), (bx, by) = pts[i], pts[(i + 1) % 4]
        d = np.minimum(d, segment(c, ax, ay, bx, by))
    d = np.minimum(d, segment(c, x0 + 4.5, 9, x0 + 19.5, 9))
    d = np.minimum(d, np.minimum(segment(c, x0 + 9, 9, x0 + 12, 21.5), segment(c, x0 + 15, 9, x0 + 12, 21.5)))
    c.paint(line(d, 0.95), WHITE)
    # penguin: a round body with a belly cut out, a beak and feet
    x0 = 48
    body = np.hypot((c.x - x0 - 12) / 6.6, (c.y - 12.5) / 8.6) - 1
    belly = np.hypot((c.x - x0 - 12) / 3.8, (c.y - 15) / 5.2) - 1
    c.paint(inside(body * 6.6, 0.6) * inside(-belly * 3.8, 0.6), WHITE)
    for fx in (-3, 3):
        c.paint(inside(np.hypot((c.x - x0 - 12 - fx) / 2.4, (c.y - 21.6) / 1.1) - 1, 0.6), WHITE)
    # warning: a rounded triangle with "!"
    x0 = 72
    pts = [(x0 + 12, 3), (x0 + 21.5, 20.5), (x0 + 2.5, 20.5)]
    d = np.full_like(c.a, 99.0)
    for i in range(3):
        (ax, ay), (bx, by) = pts[i], pts[(i + 1) % 3]
        d = np.minimum(d, segment(c, ax, ay, bx, by))
    c.paint(line(d, 1.1), WHITE)
    c.paint(line(segment(c, x0 + 12, 9.5, x0 + 12, 14), 1.1), WHITE)
    c.paint(inside(np.hypot(c.x - x0 - 12, c.y - 17.4) - 1.2, 0.6), WHITE)
    return c


PICTURES = {'round': round_box, 'edge': edge, 'shadow': shadow, 'icons': icons}


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, make in PICTURES.items():
        make().image().save(os.path.join(OUT, name + '.png'), optimize=True)
        print(name)


if __name__ == '__main__':
    main()
