"""Runs inside Blender 2.79: makes crystalempty.blend (the crystal left when
a crystal field is used up) from iceempty.blend, with each little ice cube
turned into a pyramid of the same size, standing where the cube stood.

    blender -b iceempty.blend --python make_crystalempty.py -- OUT.blend"""
import bpy, bmesh, sys

out = sys.argv[-1]
for o in list(bpy.context.scene.objects):
    if o.type != 'MESH' or not o.name.startswith('Cube'):
        continue
    xs, ys, zs = zip(*(v.co for v in o.data.vertices))
    x0, x1, y0, y1, z0, z1 = min(xs), max(xs), min(ys), max(ys), min(zs), max(zs)
    bm = bmesh.new()
    base = [bm.verts.new(c) for c in ((x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0))]
    # a little taller than the cube, so it reads as a crystal
    top = bm.verts.new(((x0 + x1) / 2, (y0 + y1) / 2, z0 + (z1 - z0) * 1.4))
    bm.faces.new(list(reversed(base)))
    for i in range(4):
        bm.faces.new((base[i], base[(i + 1) % 4], top))
    mesh = bpy.data.meshes.new(o.name + '-pyramid')
    bm.to_mesh(mesh)
    bm.free()
    for m in o.data.materials:
        mesh.materials.append(m)
    o.data = mesh
bpy.ops.wm.save_as_mainfile(filepath=out)
