"""Runs inside Blender 2.79: makes whiteforest.blend (the picture of the
White Forest map) from snowmound3.blend1: the snow penguin on its ice stand,
without the writing on the stand, in a forest of the trees of TREE.blend...
and a camera low in front of it.  Render it at frame 18.

The TREEs are those files saved by Blender 2.79 (Blender 2.79 appends
nothing from the 2.4x files).

    blender -b snowmound3.blend1 --python make_whiteforest.py -- TREE... OUT.blend"""
import bpy, math, random, sys
from mathutils import Vector

args = sys.argv[sys.argv.index('--') + 1:]
trees, out = args[:-1], args[-1]
scene = bpy.context.scene
objects = bpy.data.objects

# the writing, and the bar through it
for name in ('Font', 'Cube.001'):
    objects.remove(objects[name], do_unlink=True)

# where the trees stand round the snow penguin (which is about 7 wide and
# 8 high round 1, 0): behind it, at its sides, and at the picture's edges
PLACES = [(-7.5, 9), (-2, 12), (4, 11), (9, 7), (-11, 3), (7.5, 1),
          (-9, -4), (8.5, -4.5), (1, 17), (-14, 12), (14, 11), (7, 15),
          (12, 3), (-5, 18), (11, 19)]
rnd = random.Random(3)
ground = objects['Plane.004']
GROUND = min((ground.matrix_world * Vector(c)).z for c in ground.bound_box)
skip = {'Camera', 'Plane.004'}
for i, (x, y) in enumerate(PLACES):
    with bpy.data.libraries.load(trees[i % len(trees)]) as (src, dst):
        dst.objects = [n for n in src.objects if n not in skip and not n.startswith('Lamp')]
    tree = bpy.data.objects.new('tree', None)
    scene.objects.link(tree)
    corners = [o.matrix_world * Vector(c) for o in dst.objects for c in o.bound_box]
    # the trunk's foot: the middle of the tree, on the ground
    foot = Vector((sum(c.x for c in corners) / len(corners), sum(c.y for c in corners) / len(corners), GROUND))
    tree.location = foot
    scene.update()
    for o in dst.objects:
        scene.objects.link(o)
        if not o.parent:
            o.parent = tree
            o.matrix_parent_inverse = tree.matrix_world.inverted()
    tree.location = (x, y, GROUND)
    tree.rotation_euler = (0, 0, rnd.uniform(0, 2 * math.pi))
    tree.scale = [rnd.uniform(0.9, 1.15)] * 3

# low in front of the snow penguin, the trees round and behind it
cam = scene.camera
cam.data.type = 'PERSP'
cam.data.lens = 34
cam.location = Vector((-3.2, -9.5, -1.4))
cam.rotation_mode = 'XYZ'
cam.rotation_euler = (Vector((0.9, 0, 1.2)) - cam.location).to_track_quat('-Z', 'Y').to_euler()
cam.data.clip_end = 200
scene.frame_set(18)
bpy.ops.wm.save_as_mainfile(filepath=out)
