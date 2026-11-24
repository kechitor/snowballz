"""The wallpaper (wallpaper.png, the picture at the top of README.md): the
worker walking away with its block of ice, on snow under a cloudy sky, at
1920x1080.

    python3 tools/sprites/make_wallpaper.py BLENDS_DIR [BLENDER_2.79]

Like make_sprites.py (BLENDS_DIR, Blender 2.79, Pillow and numpy)."""
import os, sys, tempfile
import numpy as np
from PIL import Image
from make_backgrounds import ROOT, blender_run, blend_path, render, saved_by_279, sky

W, H = 1920, 1080
# behind the worker and a little to its left, low over the snow
VIEW = ['frame=13', 'camera=3,17,2.2,0.3,0,0.2,22']
HAZE = np.array([228, 228, 255])


def main():
    blends = sys.argv[1]
    blender = sys.argv[2] if len(sys.argv) > 2 else 'blender'
    tmp = tempfile.mkdtemp(prefix='snowballz-wallpaper-')
    scene = os.path.join(tmp, 'wallpaper.blend')
    blender_run(blender, blend_path(blends, 'walkanimation_worker_ice'), 'make_wallpaper_scene.py',
                saved_by_279(blender, blends, tmp, 'crystal_crises'), scene)
    full = render(blender, tmp, tmp, 'full', 'wallpaper', [W, H], [0, 0, W, H], VIEW)
    worker = render(blender, tmp, tmp, 'worker', 'wallpaper', [W, H], [0, 0, W, H], VIEW + ['hide=Plane.004'])
    # the far snow is out of the lamps' reach: fade it into a light haze
    a = np.asarray(full).astype(float)
    horizon = np.nonzero(a[:, 0, 3] > 0)[0].min()
    t = np.clip(1 - (np.arange(H) - horizon) / (0.4 * H), 0, 1)[:, None, None] ** 1.5
    a[..., :3] = a[..., :3] * (1 - t) + HAZE * t
    img = sky(W, H, 5, horizon=0.8)
    img.alpha_composite(Image.fromarray(a.astype('uint8'), 'RGBA'))
    img.alpha_composite(worker)
    img.convert('RGB').save(os.path.join(ROOT, 'wallpaper.png'), optimize=True)


if __name__ == '__main__':
    main()
