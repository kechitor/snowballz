"""Runs inside Blender 2.79: renders a .blend file's picture onto a larger
canvas, with a transparent sky.  The file's own picture (its camera's frame)
lands at X, Y, W, H on the W_OUT x H_OUT canvas; the camera sees more of the
scene round it to fill the rest.

    blender -b FILE.blend --python render_view.py -- OUT.png W_OUT H_OUT X Y W H [OPTION...]

OPTIONs:
  hide=NAME                      leave out object NAME, and all hanging on it
  frame=N                        the frame to render (else the file's own)
  camera=X,Y,Z,TX,TY,TZ,LENS     a perspective camera at X,Y,Z looking at TX,TY,TZ"""
import bpy, sys
from mathutils import Vector

args = sys.argv[sys.argv.index('--') + 1:]
out = args[0]
cw, ch, x, y, w, h = map(float, args[1:7])
options = [a.split('=', 1) for a in args[7:]]
scene = bpy.context.scene
render = scene.render
cam = scene.camera

hide = {v for k, v in options if k == 'hide'}
for o in scene.objects:
    p = o
    while p and p.name not in hide:
        p = p.parent
    if p:
        o.hide_render = True
for k, v in options:
    if k == 'frame':
        scene.frame_set(int(v))
    if k == 'camera':
        v = list(map(float, v.split(',')))
        cam.data.type = 'PERSP'
        cam.data.lens = v[6]
        cam.location = Vector(v[:3])
        cam.rotation_mode = 'XYZ'
        cam.rotation_euler = (Vector(v[3:6]) - cam.location).to_track_quat('-Z', 'Y').to_euler()

# the file's frame is fitted to the wider of its sides; keep that size for W
fw, fh = render.resolution_x * render.pixel_aspect_x, render.resolution_y * render.pixel_aspect_y
sensor = cam.data.sensor_width if fw >= fh else cam.data.sensor_width * fw / fh
cam.data.sensor_fit = 'HORIZONTAL'
cam.data.sensor_width = sensor * cw / w
# (shift is in parts of the canvas' larger side)
cam.data.shift_x += (cw / 2 - (x + w / 2)) / max(cw, ch)
cam.data.shift_y += (y + h / 2 - ch / 2) / max(cw, ch)

render.resolution_x, render.resolution_y, render.resolution_percentage = int(cw), int(ch), 100
render.pixel_aspect_x = render.pixel_aspect_y = 1
render.use_border = False
render.alpha_mode = 'TRANSPARENT'
render.use_compositing = False
render.image_settings.file_format = 'PNG'
render.image_settings.color_mode = 'RGBA'
render.filepath = out
bpy.ops.render.render(write_still=True)
