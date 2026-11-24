"""The penguin sprite sheets, trees, ice pinnacles, igloo, crystals, frozen
penguins, markers and snow walls, rendered from the original Blender files at 4 times the old
size; and the fish holes (no Blender files) from painted pictures.

    python3 tools/sprites/make_sprites.py BLENDS_DIR [BLENDER_2.79]

BLENDS_DIR holds the original .blend files.  They are from Blender 2.42-2.45
and need Blender 2.79 (the last with Blender Internal and the old animation
converter).  crystalempty.blend and snowballerfrozen.blend are made there
from the originals if they are missing (make_crystalempty.py,
make_snowballerfrozen.py).  Needs Pillow and numpy.  Writes into data/; the
game draws each picture at its old size, and picks a smaller copy when
zoomed out.

A sheet has one row per direction and 10 frames, plus a standing frame in
the 11th column for the walk sheets, like data/convert.py in the Python
version.  The blend files animate 20 frames per direction; the first 10 of
each are one cycle."""
import collections, os, subprocess, sys, tempfile, threading
from concurrent.futures import ThreadPoolExecutor
import numpy as np
from PIL import Image, ImageDraw

SCALE = 4
HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, '..', '..', 'data')

# the first frame of each direction, in the game's row order:
# (0,1), (-1,1), (-1,0), (-1,-1), (0,-1), (1,-1), (1,0), (1,1)
STARTS = [1, 21, 41, 61, 81, 101, 121, 151]

# set up to render at 4 times the game's size already
LARGE = {'crystal', 'worker_gathering_ice'}

# sheet: (blend file, mode, idle frames from (blend file, mode) or None)
SHEETS = {
    'walk': ('walkanimation', 'base', ('walkanimation_penguin_idle', 'base')),
    'workerwalk': ('walkanimation_worker_outfit', 'asis', ('walkanimation_worker_outfit', 'asis')),
    'snowballerwalk': ('penguin_snowballer_walkanimation', 'asis', ('walkanimation_snowballer_idle', 'asis')),
    'throw': ('throwanimation_snowballer', 'base', None),
    'snowballerthrow': ('throwanimation_snowballer_outfit', 'asis', None),
    'gathersnow': ('snowballer_gather', 'base', None),
    'snowballergathersnow': ('snowballer_gather_outfit', 'asis', None),
    'unloadfish': ('unload_worker_fish', 'base', None),
    'workerunloadfish': ('unload_worker_fish_outfit', 'asis', None),
    'unloadcrystal': ('unload_worker_crystal', 'base', None),
    'workerunloadcrystal': ('unload_worker_crystal', 'outfit', None),
    'gatherfish': ('worker_fishing', 'base', None),
    'workergatherfish': ('worker_fishing_outfit', 'asis', None),
    'gathercrystal': ('worker_gathering_ice', 'base', None),
    'workergathercrystal': ('worker_gathering_ice', 'outfit', None),
    'walkfish': ('walkanimation_worker_fish', 'base', None),
    'workerwalkfish': ('walkanimation_worker_fish', 'outfit', None),
    'walkcrystal': ('walkanimation_worker_crystal', 'base', None),
}

# picture: (size of the old picture, [(blend file, x, y of its old top left corner, frame)])
# the offsets put each render where the old picture had it
PICTURES = {
    'trees': ((256, 128), [('tree1', 0, -2, 1), ('tree3', 62, -3, 1), ('tree2', 0, 64, 1), ('tree', 64, 64, 1)]),
    'icepinnacles': ((256, 128), [('icespiral', -5, 0, 1), ('icespiral1', 57, 0, 1), ('icespiral2', -12, 67, 1)]),
    'igloo': ((128, 128), [('igloo', 0, 0, 1)]),
    'crystal': ((32, 32), [('crystal', 0, -1, 1)]),
    'crystalempty': ((32, 32), [('crystalempty', 0, -1, 1)]),
    'frozenworker': ((32, 32), [('workerfrozen', 0, 0, 1)]),
    'frozensnowballer': ((32, 32), [('snowballerfrozen', 0, 0, 15)]),
}

# the outfits of the frozen penguins: (blend file, frame)
FROZEN_OUTFITS = {
    'frozenworkeroutfit': ('workerfrozen', 1),
    'frozensnowballeroutfit': ('snowballerfrozen', 15),
}

# the white cone that marks orders and igloos (flag.blend), and the symbols
# drawn on it, in the old pictures' pixels: marker: (render mode, (x, y) of
# the cone, [(colour, polygon or ('ring', x, y, inner radius, outer radius))])
ARROW_TOP = [(11, 13.5), (1.5, 23), (18, 23), (18, 20), (11, 20)]
ARROW_BOTTOM = [(11, 32.5), (1.5, 23), (18, 23), (18, 26), (11, 26)]
CHEVRON = [(26.5, 18), (31, 22.5), (31, 25), (22, 25), (22, 22.5)]
MARKERS = {
    'marker': ('base', (0, 0), []),
    'marker_leave': ('base', (9, 1), [(255, ARROW_TOP), (199, ARROW_BOTTOM)]),
    'marker_gather': ('base', (0, 0), [(240, ('ring', 27, 26, 3, 4)), (240, ('ring', 27, 26, 0, 2))]),
    'marker_run': ('base', (0, 0), [(240, CHEVRON), (240, [(x, y + 6) for x, y in CHEVRON])]),
    'jeopardy_marker': ('crystal', (0, 0), []),
}

# pictures with no Blender file, painted at 16 times the old size (cut out
# of a picture of both, and fitted to where the old ones were)
PAINTED = ['fish', 'fishempty']


def blender_run(blender, blend, script, *args):
    subprocess.run([blender, '-b', blend, '--python', os.path.join(HERE, script), '--', *args],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def made_blends(blender, blends, tmp):
    """The .blend files that this project made from the originals."""
    out = os.path.join(blends, 'crystalempty.blend')
    if not os.path.exists(out):
        blender_run(blender, os.path.join(blends, 'iceempty.blend'), 'make_crystalempty.py', out)
    out = os.path.join(blends, 'snowballerfrozen.blend')
    if not os.path.exists(out):
        # the snowballer's outfit can only be taken from a file saved by 2.79
        saved = os.path.join(tmp, 'walkanimation_snowballer.blend')
        subprocess.run([blender, '-b', os.path.join(blends, 'walkanimation_snowballer.blend'), '--python-expr',
                        'import bpy; bpy.ops.wm.save_as_mainfile(filepath=%r)' % saved],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        blender_run(blender, os.path.join(blends, 'workerfrozen.blend'), 'make_snowballerfrozen.py', saved, out)


# one render at a time into each folder (two pictures can share a render)
locks = collections.defaultdict(threading.Lock)
locks_lock = threading.Lock()


def render(blender, blends, tmp, blend, mode, frames):
    out = os.path.join(tmp, '%s-%s' % (blend, mode))
    with locks_lock:
        lock = locks[out]
    with lock:
        os.makedirs(out, exist_ok=True)
        todo = [f for f in frames if not os.path.exists('%s/%04d.png' % (out, f))]
        if todo:
            scale = 1 if blend in LARGE else SCALE
            blender_run(blender, os.path.join(blends, blend + '.blend'), 'render.py', out, mode, str(scale), ','.join(map(str, todo)))
    return lambda f: Image.open('%s/%04d.png' % (out, f)).convert('RGBA')


def sheet(blender, blends, tmp, name):
    blend, mode, idle = SHEETS[name]
    cell = 32 * SCALE
    frames = [s + i for s in STARTS for i in range(10)]
    get = render(blender, blends, tmp, blend, mode, frames)
    get_idle = idle and render(blender, blends, tmp, idle[0], idle[1], STARTS)
    img = Image.new('RGBA', (512 * SCALE, 256 * SCALE))
    for row, s in enumerate(STARTS):
        for i in range(10):
            img.paste(get(s + i), (i * cell, row * cell))
        if get_idle:
            img.paste(get_idle(s), (10 * cell, row * cell))
    return img


def picture(blender, blends, tmp, name):
    (w, h), parts = PICTURES[name]
    img = Image.new('RGBA', (w * SCALE, h * SCALE))
    for blend, x, y, frame in parts:
        r = render(blender, blends, tmp, blend, 'base', [frame])(frame)
        img.alpha_composite(r.crop((max(0, -x * SCALE), max(0, -y * SCALE), r.width, r.height)), (max(0, x * SCALE), max(0, y * SCALE)))
    return img


def frozen_outfit(blender, blends, tmp, name):
    """The outfit of a penguin in an ice block, for the game to tint and draw
    over the frozen penguin: where the render with the (grey) outfit differs
    from the one without, the pixels of the render with it.  Through the ice
    it differs less, so the team colour shows faintly there."""
    blend, frame = FROZEN_OUTFITS[name]
    bare = np.asarray(render(blender, blends, tmp, blend, 'base', [frame])(frame), float) / 255
    dressed = np.asarray(render(blender, blends, tmp, blend, 'asis', [frame])(frame), float) / 255
    premul = lambda a: a[..., :3] * a[..., 3:]
    diff = np.abs(premul(dressed) - premul(bare)).max(-1)
    out = dressed.copy()
    out[..., 3] = np.clip(diff * 3, 0, 1) * dressed[..., 3]
    return Image.fromarray((out * 255).round().astype('uint8'), 'RGBA')


def marker(blender, blends, tmp, name):
    mode, (x, y), symbols = MARKERS[name]
    img = Image.new('RGBA', (32 * SCALE, 32 * SCALE))
    img.alpha_composite(render(blender, blends, tmp, 'flag', mode, [1])(1), (x * SCALE, y * SCALE))
    # drawn 4 times bigger again and shrunk, for smooth edges
    k = SCALE * 4
    layer = Image.new('RGBA', (32 * k, 32 * k))
    draw = ImageDraw.Draw(layer)
    for grey, shape in symbols:
        if shape[0] == 'ring':
            _, cx, cy, inner, outer = shape
            draw.ellipse([(cx - outer) * k, (cy - outer) * k, (cx + outer) * k, (cy + outer) * k], fill=(grey, grey, grey, 255))
            if inner:
                draw.ellipse([(cx - inner) * k, (cy - inner) * k, (cx + inner) * k, (cy + inner) * k], fill=(0, 0, 0, 0))
        else:
            draw.polygon([(px * k, py * k) for px, py in shape], fill=(grey, grey, grey, 255))
    img.alpha_composite(layer.resize(img.size, Image.LANCZOS))
    return img


def walls(blender, blends, tmp, name):
    """The pieces of the snow walls (render_walls.py): 16 columns, 6 rows."""
    out = os.path.join(tmp, 'walls')
    os.makedirs(out, exist_ok=True)
    blender_run(blender, os.path.join(blends, 'wall.blend'), 'render_walls.py', os.path.join(blends, 'loser.blend'), out, str(SCALE))
    cell = 64 * SCALE
    img = Image.new('RGBA', (16 * cell, 6 * cell))
    for row in range(6):
        for col in range(16):
            img.paste(Image.open('%s/%d-%d.png' % (out, row, col)), (col * cell, row * cell))
    return img


def painted(blender, blends, tmp, name):
    return Image.open(os.path.join(HERE, 'painted', name + '.png')).convert('RGBA').resize((32 * SCALE, 32 * SCALE), Image.LANCZOS)


def main():
    blends = sys.argv[1]
    blender = sys.argv[2] if len(sys.argv) > 2 else 'blender'
    tmp = tempfile.mkdtemp(prefix='snowballz-sprites-')
    made_blends(blender, blends, tmp)
    jobs = ([(sheet, n) for n in SHEETS] + [(picture, n) for n in PICTURES] +
            [(frozen_outfit, n) for n in FROZEN_OUTFITS] + [(marker, n) for n in MARKERS] + [(painted, n) for n in PAINTED] + [(walls, 'walls')])
    with ThreadPoolExecutor(os.cpu_count()) as pool:
        for name, img in pool.map(lambda j: (j[1], j[0](blender, blends, tmp, j[1])), jobs):
            img.save(os.path.join(DATA, name + '.png'), optimize=True)
            print(name, img.size)


if __name__ == '__main__':
    main()
