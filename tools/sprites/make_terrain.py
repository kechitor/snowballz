"""The ground's textures, data/snow.png and data/ice.png: seamless, 1024
pixels for the 256 world pixels each covers (4 times the old snow's detail,
twice the old ice's), with the old pictures' average colours.

    python3 tools/sprites/make_terrain.py

Needs Pillow and numpy."""
import os
import numpy as np
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, '..', '..')
N = 1024


def noise(rng, low, high, slope=1.0):
    """Seamless noise of spatial frequencies `low` to `high` (cycles across
    the picture), louder towards the low ones; scaled to -1..1."""
    f = np.hypot(*np.meshgrid(np.fft.fftfreq(N) * N, np.fft.fftfreq(N) * N))
    band = (f >= low) & (f <= high)
    spectrum = np.fft.fft2(rng.standard_normal((N, N))) * band / np.maximum(f, 1) ** slope
    n = np.real(np.fft.ifft2(spectrum))
    return n / np.abs(n).max()


def blur(a, sigma):
    """Gaussian blur that wraps round the edges (keeps the picture seamless)."""
    f = np.fft.fftfreq(N)
    g = np.exp(-2 * (np.pi * sigma) ** 2 * (f[:, None] ** 2 + f[None, :] ** 2))
    return np.real(np.fft.ifft2(np.fft.fft2(a) * g))


def wrapped(draw_lines, width=1):
    """A seamless layer of lines: each line is drawn again one picture
    width / height over, so what leaves one side comes in at the other."""
    layer = Image.new('L', (N, N))
    d = ImageDraw.Draw(layer)
    for pts, value in draw_lines:
        for ox in (-N, 0, N):
            for oy in (-N, 0, N):
                d.line([(x + ox, y + oy) for x, y in pts], fill=value, width=width)
    return np.asarray(layer).astype(float) / 255


def snow(rng):
    drifts = noise(rng, 1, 6, 1.0)             # soft rises and dips
    grain = noise(rng, 60, 400, 0.3)           # packed snow
    lumps = noise(rng, 8, 40, 0.8)
    v = 236 + 2.6 * drifts + 1.6 * lumps + 1.8 * grain
    rgb = np.stack([v - 1.2 * (drifts < 0) * -drifts, v - 0.6 * (drifts < 0) * -drifts, v + 1.5 * (drifts < 0) * -drifts], -1)
    # sparkles: a few single bright crystals
    sparkle = np.zeros((N, N))
    ys, xs = rng.integers(0, N, 900), rng.integers(0, N, 900)
    sparkle[ys, xs] = rng.uniform(8, 18, 900)
    sparkle = blur(sparkle, 0.7) * 2.2
    rgb = rgb + sparkle[..., None]
    return rgb


def ice(rng):
    cloud = noise(rng, 1, 8, 1.0)              # milky and clear patches
    fine = noise(rng, 80, 450, 0.2)
    base = np.array([192.0, 185.0, 250.0])
    rgb = base + cloud[..., None] * np.array([4, 4, 2]) + fine[..., None] * np.array([2.5, 2.5, 1.5])
    # skate scratches: long faint straight lines
    lines = []
    for i in range(70):
        x, y, a, l = rng.uniform(0, N), rng.uniform(0, N), rng.uniform(0, np.pi), rng.uniform(80, 400)
        lines.append(([(x, y), (x + l * np.cos(a), y + l * np.sin(a))], int(rng.uniform(70, 160))))
    scratches = blur(wrapped(lines), 0.6)
    rgb = rgb + scratches[..., None] * np.array([26, 26, 6])
    # cracks: a few jagged lines, dark with a bright edge
    cracks = []
    for i in range(3):
        x, y, a = rng.uniform(0, N), rng.uniform(0, N), rng.uniform(0, 2 * np.pi)
        pts = [(x, y)]
        for s in range(int(rng.integers(6, 14))):
            a += rng.uniform(-0.3, 0.3)
            x, y = x + 18 * np.cos(a), y + 18 * np.sin(a)
            pts.append((x, y))
        cracks.append((pts, 255))
    crack = blur(wrapped(cracks, 2), 0.8)
    edge = np.roll(np.roll(crack, 2, 0), 2, 1)
    rgb = rgb - crack[..., None] * np.array([26, 26, 12]) + edge[..., None] * np.array([16, 16, 4])
    # frost: soft white specks
    frost = noise(rng, 20, 90, 0.5)
    rgb = rgb + np.clip(frost - 0.55, 0, 1)[..., None] * np.array([90, 90, 20])
    return rgb


def save(rgb, name, mean):
    # keep the old picture's average colour, so the maps look the same from afar
    rgb = rgb - rgb.reshape(-1, 3).mean(0) + np.array(mean)
    img = Image.fromarray(np.clip(rgb + 0.5, 0, 255).astype('uint8'), 'RGB').convert('RGBA')
    img.save(os.path.join(ROOT, 'data', name), optimize=True)


def main():
    rng = np.random.default_rng(11)
    save(snow(rng), 'snow.png', (236.2, 236.2, 236.2))
    save(ice(rng), 'ice.png', (192.2, 185.2, 250.6))


if __name__ == '__main__':
    main()
