"""Runs inside Blender 2.79: makes the scene of the wallpaper (wallpaper.png)
from walkanimation_worker_ice.blend: the worker walking away with its block
of ice, on the snow of crystal_crises.blend (GROUND, saved by Blender 2.79),
made wide enough to reach the horizon.

    blender -b walkanimation_worker_ice.blend --python make_wallpaper_scene.py -- GROUND OUT.blend"""
import bpy, sys

ground, out = sys.argv[-2:]
scene = bpy.context.scene
objects = bpy.data.objects
objects.remove(objects['Plane.004'], do_unlink=True)
with bpy.data.libraries.load(ground) as (src, dst):
    dst.objects = ['Plane.004']
for o in dst.objects:
    scene.objects.link(o)
    o.scale = (8, 8, 1)
scene.camera.data.clip_end = 300
bpy.ops.wm.save_as_mainfile(filepath=out)
