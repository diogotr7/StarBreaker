"""Blender 5.1 reference for corner fan-space custom normals (fixture generator)."""
import bpy, sys, math, random
out = sys.argv[sys.argv.index('--')+1]
random.seed(7)
bpy.ops.wm.read_factory_settings(use_empty=True)
verts, tris = [], []
# 1) closed perturbed UV sphere (cyclic fans, poles)
SEG, RING = 8, 5
def jitter(v, s=0.03): return tuple(c + random.uniform(-s, s) for c in v)
top = len(verts); verts.append(jitter((0, 0, 1)))
for r in range(1, RING):
    th = math.pi * r / RING
    for s in range(SEG):
        ph = 2 * math.pi * s / SEG
        verts.append(jitter((math.sin(th)*math.cos(ph), math.sin(th)*math.sin(ph), math.cos(th))))
bot = len(verts); verts.append(jitter((0, 0, -1)))
ring = lambda r, s: 1 + (r - 1) * SEG + (s % SEG)
for s in range(SEG):
    tris.append((top, ring(1, s), ring(1, s+1)))
    tris.append((bot, ring(RING-1, s+1), ring(RING-1, s)))
for r in range(1, RING-1):
    for s in range(SEG):
        a, b, c, d = ring(r, s), ring(r+1, s), ring(r+1, s+1), ring(r, s+1)
        tris += [(a, b, c), (a, c, d)]
# 2) open grid with a split seam column (open fans, boundary corners)
base = len(verts); W, H = 5, 4
for y in range(H):
    for x in range(W):
        verts.append(jitter((3 + x * 0.5, y * 0.5, 0.2 * math.sin(x + y)), 0.05))
seam = len(verts)
for y in range(H):  # duplicate column x=2 (seam)
    verts.append(verts[base + y * W + 2])
g = lambda x, y, right=False: (seam + y) if (x == 2 and right) else base + y * W + x
for y in range(H - 1):
    for x in range(W - 1):
        r = x >= 2
        a, b, c, d = g(x, y, r), g(x+1, y, r), g(x+1, y+1, r), g(x, y+1, r)
        tris += [(a, b, c), (a, c, d)]
# 3) non-manifold fin on an interior grid edge, 4) flipped winding neighbour, 5) zero-area triangle
fin = len(verts); verts.append((3.6, 0.6, 1.0))
tris.append((g(1, 1), g(2, 1), fin))
fa, fb, fc, fd = [len(verts) + i for i in range(4)]
verts += [(6, 0, 0), (7, 0, 0.1), (7, 1, 0), (6, 1, 0.2)]
tris += [(fa, fb, fc), (fa, fd, fc)]  # second triangle wound the other way across edge fa-fc
za, zb, zc = [len(verts) + i for i in range(3)]
verts += [(8, 0, 0), (9, 0, 0), (10, 0, 0)]
tris.append((za, zb, zc))
normals = []
for i, v in enumerate(verts):
    n = [v[0] - (0 if i < base else v[0]) + random.uniform(-0.3, 0.3), v[1] * (1 if i < base else 0) + random.uniform(-0.3, 0.3), v[2] * (1 if i < base else 0) + (0 if i < base else 1) + random.uniform(-0.3, 0.3)]
    l = math.sqrt(sum(c*c for c in n)); normals.append([c / l for c in n])
me = bpy.data.meshes.new("fixture")
me.from_pydata(verts, [], tris)
me.update()
me.shade_smooth()
me.normals_split_custom_set_from_vertices(normals)
attr = me.attributes["custom_normal"]
assert (attr.domain, attr.data_type) == ('CORNER', 'INT16_2D'), (attr.domain, attr.data_type)
sharp_e = me.attributes.get("sharp_edge")
assert sharp_e is None or not any(d.value for d in sharp_e.data)
sharp_f = me.attributes.get("sharp_face")
assert sharp_f is None or not any(d.value for d in sharp_f.data)
pos = [v.co for v in me.vertices]
with open(out, "w") as f:
    f.write("# Blender 5.1 normals_split_custom_set_from_vertices reference; see custom_normals.rs\n")
    for p, n in zip(pos, normals):
        f.write("v %r %r %r %r %r %r\n" % (p[0], p[1], p[2], n[0], n[1], n[2]))
    for t in tris:
        f.write("t %d %d %d\n" % t)
    for d, cn in zip(attr.data, me.corner_normals):
        f.write("c %d %d %r %r %r\n" % (d.value[0], d.value[1], cn.vector[0], cn.vector[1], cn.vector[2]))
print("verts", len(verts), "tris", len(tris))
