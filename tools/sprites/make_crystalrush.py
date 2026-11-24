"""Runs inside Blender 2.79: makes smallcrystalrush.blend (the picture of the
Small Crystal Rush map) from crystal.blend: a worker with its pick (from
worker_gathering_ice.blend) standing on snow (the ground of
crystal_crises.blend), facing the crystal.  Render it at frame 1.

WORKER and GROUND are those files saved by Blender 2.79 (appending straight
from the 2.4x files loses the animation).

    blender -b crystal.blend --python make_crystalrush.py -- WORKER GROUND OUT.blend"""
import bpy, math, sys
from mathutils import Matrix, Vector

worker, ground, out = sys.argv[-3:]
scene = bpy.context.scene
objects = bpy.data.objects

# the crystal, in front of the worker (who faces -y at frame 1)
objects.remove(objects['Plane.004'], do_unlink=True)
for o in list(scene.objects):
    if o.type == 'MESH':
        o.location += Vector((-1.5, -10.5, 0))


def append(path, names):
    with bpy.data.libraries.load(path) as (src, dst):
        dst.objects = [n for n in src.objects if names(n)]
    for o in dst.objects:
        scene.objects.link(o)


skip = {'Camera', 'Plane.004'}
append(worker, lambda n: n not in skip and not n.startswith('Lamp'))
append(ground, lambda n: n == 'Plane.004')


# the worker's file plays its frames 4 times as fast (crystal.blend: 2.5)
scene.render.frame_map_old, scene.render.frame_map_new = 16, 4


def flat_feet(ground):
    """The feet are boards tilted steeply up, which look like flat feet only
    from the game's high camera: turn them down flat, shorter, and set each
    on the snow."""
    arm = objects['Armature']
    scene.objects.active = arm
    bpy.ops.object.mode_set(mode='EDIT')
    for side in 'LR':
        arm.data.edit_bones['foot1.' + side].use_connect = False
    bpy.ops.object.mode_set(mode='OBJECT')
    action = arm.animation_data.action
    for fc in list(action.fcurves):
        if fc.data_path.startswith('pose.bones["foot'):
            action.fcurves.remove(fc)
    scene.frame_set(1)

    def low(foot):
        scene.update()
        mesh = foot.to_mesh(scene, True, 'RENDER')
        z = min((foot.matrix_world * v.co).z for v in mesh.vertices)
        bpy.data.meshes.remove(mesh)
        return z

    for name, side in (('Plane', 'R'), ('Plane.001', 'L')):
        foot, bone = objects[name], arm.pose.bones['foot1.' + side]
        h = bone.head.copy()
        bone.matrix = Matrix.Translation(h) * Matrix.Rotation(math.radians(69), 4, 'X') * Matrix.Translation(-h) * bone.matrix
        scene.update()
        bone.scale = (1, 0.55, 1)
        # (bone.location[1] moves the foot mostly up and down)
        for i in range(4):
            z = low(foot)
            bone.location[1] += 0.1
            step = (low(foot) - z) / 0.1
            bone.location[1] += (ground + 0.03 - z) / step - 0.1


flat_feet(objects['Plane.004'].matrix_world.translation.z)

# low, behind the worker's shoulder, with the sky above the snow
cam = scene.camera
cam.data.type = 'PERSP'
cam.data.lens = 32
cam.location = Vector((13, 10, -1.2))
cam.rotation_mode = 'XYZ'
cam.rotation_euler = (Vector((-1, -4.5, 1.8)) - cam.location).to_track_quat('-Z', 'Y').to_euler()
scene.frame_set(1)
bpy.ops.wm.save_as_mainfile(filepath=out)
