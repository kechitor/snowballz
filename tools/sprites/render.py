"""Runs inside Blender 2.79 (Blender Internal; newer versions lose the 2.4x
animations and materials).  Renders frames of one .blend file, SCALE times
the size of the game's original sprites, with a transparent background:

    blender -b FILE.blend --python render.py -- OUT MODE SCALE FRAMES

MODE says what to draw:
  base    the penguin without its team-coloured outfit
  outfit  only the outfit, grey (the game tints it), with the rest of the
          penguin hiding the parts of the outfit behind it
  asis    as the file is set up (outfit files made for the game), with
          the outfit grey
  crystal everything in the file's crystal material (flag.blend: the
          marker of an igloo being taken)

FRAMES is a comma separated list; each is written to OUT/NNNN.png."""
import bpy, sys

out, mode, scale, frames = sys.argv[-4], sys.argv[-3], float(sys.argv[-2]), sys.argv[-1]
scene = bpy.context.scene
render = scene.render


def shown(o):
    return any(a and b for a, b in zip(o.layers, scene.layers))


def materials(o):
    return [s.material for s in o.material_slots if s.material]


def is_outfit(o):
    return any(m.name.startswith('cap') for m in materials(o))


def is_ground(o):
    """The plane that catches the penguin's shadow."""
    ms = materials(o)
    return bool(ms) and all(m.use_only_shadow for m in ms)


# the outfit, grey so the game can multiply it by the team colour
grey = bpy.data.materials.new('outfit-grey')
grey.diffuse_color = (0.7, 0.7, 0.7)
grey.specular_intensity = 0
# invisible, but hides what is behind it (as in the game's outfit files)
mask = bpy.data.materials.new('outfit-mask')
mask.use_only_shadow = True
mask.shadow_only_type = 'SHADOW_ONLY_OLD'
mask.use_raytrace = False
mask.use_shadows = False
mask.use_cast_buffer_shadows = False
mask.use_transparency = True
mask.transparency_method = 'MASK'
mask.alpha = 0.3
mask.specular_intensity = 0

for o in scene.objects:
    outfit = is_outfit(o)
    if mode == 'base' and outfit:
        o.hide_render = True
    if mode in ('outfit', 'asis') and outfit:
        if mode == 'outfit' and not shown(o):
            # the worker's cap is sometimes left on a hidden layer
            o.layers = [a or b for a, b in zip(o.layers, scene.layers)]
        for s in o.material_slots:
            s.material = grey
    if mode == 'crystal' and not is_ground(o):
        for s in o.material_slots:
            s.material = bpy.data.materials['crystal']
    if mode == 'outfit' and not outfit and o.type in ('MESH', 'META', 'CURVE', 'SURFACE'):
        if is_ground(o):
            o.hide_render = True
        elif not o.material_slots:
            # drawn with the default material (the head of the pick)
            o.data.materials.append(mask)
        else:
            for s in o.material_slots:
                s.material = mask

w = render.resolution_x * render.resolution_percentage / 100
h = render.resolution_y * render.resolution_percentage / 100
render.resolution_x, render.resolution_y, render.resolution_percentage = round(w * scale), round(h * scale), 100
render.alpha_mode = 'TRANSPARENT'
render.use_compositing = False
render.image_settings.file_format = 'PNG'
render.image_settings.color_mode = 'RGBA'
for f in frames.split(','):
    scene.frame_set(int(f))
    render.filepath = '%s/%04d.png' % (out, int(f))
    bpy.ops.render.render(write_still=True)
