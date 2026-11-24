"""Runs inside Blender 2.79: renders the pieces of the snow walls (the cells
of data/walls.png, see src/wall.rs) with wall.blend's lights, camera angle
and snow, and the ice of ICE.blend (loser.blend: the ice block).

    blender -b wall.blend --python render_walls.py -- ICE.blend OUT SCALE

Writes OUT/ROW-COL.png, each 64*SCALE square.  A cell holds one arm of a
wall tile, from the tile's middle (at 32, 40 in the cell) towards a
neighbour: columns E, SE, S, SW, W, NW, N, NE, then the same reaching back
a little past the middle (for corners).  Rows: snow bricks one, two and
three courses high, three courses battered, ice, battered ice."""
import bpy, math, random, sys
from mathutils import Vector

args = sys.argv[sys.argv.index('--') + 1:]
ice_file, out, scale = args[0], args[1], int(args[2])
scene = bpy.context.scene
objects = bpy.data.objects

# sizes in the game's pixels; one tile (32 pixels) is as wide as the
# penguins' tile in their files
U = 8.82758617401123 / 32
TILT = math.cos(math.radians(45))  # the camera looks down at 45 degrees
THICK = 9
HIGH = 17          # 3 courses (12 pixels high on the screen)
BRICK = 11
GAP = 0.6
ARMS = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)]

snow = bpy.data.materials['snow']
# (its speckles are made for big lumps of snow: faint on bricks)
for t in snow.texture_slots:
    if t:
        t.diffuse_color_factor = 0.25
with bpy.data.libraries.load(ice_file) as (src, dst):
    dst.materials = ['ice']
ice = dst.materials[0]

objects.remove(objects['Cube'], do_unlink=True)
# the ground (catching the shadows), where the lights were set up for
ground = objects['Plane.004']
BASE = Vector((0, 0, min((ground.matrix_world * Vector(c)).z for c in ground.bound_box)))
# the light that casts shadows, moved 6 times as far (as bright there):
# shorter shadows, so a tile's shadow hardly falls on the wall next to it
lamp = objects['Lamp.006']
lamp.location = BASE + (lamp.location - BASE) * 6
lamp.data.distance *= 6

cam = scene.camera
cam.data.type = 'ORTHO'
cam.data.ortho_scale = 64 * U
cam.rotation_mode = 'XYZ'
cam.rotation_euler = (math.radians(45), 0, 0)
cam.location = BASE + Vector((0, -math.sin(math.radians(45)), math.cos(math.radians(45)))) * 40
# the tile's middle 8 pixels below the cell's middle
cam.data.shift_x = 0
cam.data.shift_y = 8 / 64

render = scene.render
render.resolution_x = render.resolution_y = 64 * scale
render.resolution_percentage = 100
render.alpha_mode = 'TRANSPARENT'
render.use_compositing = False
render.image_settings.file_format = 'PNG'
render.image_settings.color_mode = 'RGBA'

cube = bpy.data.meshes.new('brick')
cube.from_pydata([(x, y, z) for x in (0, 1) for y in (-0.5, 0.5) for z in (0, 1)], [],
                 [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)])
made = []


def world(gx, gy):
    """A point on the ground, in the game's pixels, to Blender's units:
    seen at 45 degrees, the ground's depth looks shorter."""
    return Vector((gx * U, -gy * U / TILT, 0))


def block(a, b, start, end, z0, z1, material, turn=0.0):
    """A block along the arm from a to b: from `start` to `end` of the way,
    z0 to z1 pixels high."""
    d = b - a
    o = objects.new('block', cube.copy())
    o.data.materials.append(material)
    o.location = BASE + a + d * start + Vector((0, 0, z0 * U))
    o.rotation_euler = (0, 0, math.atan2(d.y, d.x) + turn)
    o.scale = (d.length * (end - start), THICK * U, (z1 - z0) * U)
    bevel = o.modifiers.new('bevel', 'BEVEL')
    bevel.width = 0.5 * U
    bevel.segments = 2
    scene.objects.link(o)
    made.append(o)


def arm(col, row):
    dx, dy = ARMS[col % 8]
    n = math.hypot(dx, dy)
    back = THICK / 2 if col >= 8 else 0
    a = world(-dx / n * back, -dy / n * back)
    b = world(dx * 16, dy * 16)
    rnd = random.Random(col * 10 + row)
    if row >= 4:
        # ice: one slab; battered, a piece knocked out of its top
        if row == 4:
            block(a, b, 0, 1, 0, HIGH, ice)
        else:
            cut = rnd.uniform(0.35, 0.65)
            block(a, b, 0, cut, 0, HIGH, ice)
            block(a, b, cut, 1, 0, HIGH * rnd.uniform(0.45, 0.65), ice)
        return
    courses = {0: 1, 1: 2}.get(row, 3)
    length = (b - a).length / U
    count = max(1, round(length / BRICK))
    gap = GAP * U / (b - a).length
    for c in range(courses):
        z0, z1 = c * HIGH / 3, (c + 1) * HIGH / 3 - GAP
        cuts = sorted({0, 1} | {min(1, max(0, (i + 0.5 * (c % 2)) / count)) for i in range(1, count + 1)})
        for s, e in zip(cuts, cuts[1:]):
            if e - s < 0.05:
                continue
            # battered: bricks knocked off the top, the rest askew
            if row == 3 and c == 2 and rnd.random() < 0.6:
                continue
            if row == 3 and c == 1 and rnd.random() < 0.2:
                continue
            turn = rnd.uniform(-0.12, 0.12) if row == 3 else 0
            block(a, b, s + gap, e - gap, z0, z1, snow, turn)


for row in range(6):
    for col in range(16):
        arm(col, row)
        render.filepath = '%s/%d-%d.png' % (out, row, col)
        bpy.ops.render.render(write_still=True)
        for o in made:
            objects.remove(o, do_unlink=True)
        made.clear()
