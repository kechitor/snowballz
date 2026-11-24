"""Runs inside Blender 2.79: makes snowballerfrozen.blend from
workerfrozen.blend: the worker's cap swapped for the snowballer's sash and
headband, and the ice block turned the other way, so frozen workers and
snowballers look different.  Render it at frame 15 (the penguin facing the
way the old frozen sprite did).

SNOWBALLER is walkanimation_snowballer.blend saved by Blender 2.79 (appending
straight from the 2.4x file loses the animation).

    blender -b workerfrozen.blend --python make_snowballerfrozen.py -- SNOWBALLER OUT.blend"""
import bpy, math, sys

source, out = sys.argv[-2], sys.argv[-1]
objects = bpy.data.objects
body = objects['Armature']

objects.remove(objects['Circle'], do_unlink=True)
objects['Cube'].rotation_euler[2] = math.radians(-25)

# how the outfit's bones hang on the snowballer's skeleton: (parent bone,
# parent inverse matrix), by number of bones (loading renames them)
with bpy.data.libraries.load(source) as (src, dst):
    dst.objects = ['Armature.001', 'Armature.002']
hang = {}
for o in dst.objects:
    hang[len(o.data.bones)] = (o.parent_bone if o.parent_type == 'BONE' else None, o.matrix_parent_inverse.copy())
    objects.remove(o, do_unlink=True)

before = set(objects)
bpy.ops.wm.append(directory=source + '/Object/', files=[{'name': 'Plane.005'}, {'name': 'Circle'}])
for o in set(objects) - before:
    if o.type != 'ARMATURE':
        continue
    bone, inverse = hang[len(o.data.bones)]
    o.parent = body
    if bone:
        o.parent_type = 'BONE'
        o.parent_bone = bone
    o.matrix_parent_inverse = inverse
bpy.ops.wm.save_as_mainfile(filepath=out)
