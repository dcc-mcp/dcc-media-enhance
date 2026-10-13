"""Original Blender adapter: read an existing EXR; never reopen a production scene.

This file is embedded in the Rust executable. No Blender or third-party implementation
is copied or bundled. The installed Blender supplies its own denoiser.
"""
import json
import math
import struct
import sys
from pathlib import Path

import bpy

cfg = json.loads(Path(sys.argv[sys.argv.index("--") + 1]).read_text())
source, target = Path(cfg["input"]), Path(cfg["output"])
if not source.is_file() or target.exists():
    raise ValueError("Source must exist and staged output must not exist")

# Read a bounded flat EXR header. Pixel decoding and opaque-alpha checks follow.
with source.open("rb") as stream:
    header = stream.read(65536)
if header[:4] != b"\x76\x2f\x31\x01":
    raise ValueError("Not OpenEXR")
version = struct.unpack_from("<I", header, 4)[0]
if version & (0x800 | 0x1000):
    raise ValueError("Multipart/deep EXR not supported by this initial Blender adapter")
pos, dimensions = 8, None
def text():
    global pos
    end = header.find(b"\0", pos)
    if end < 0 or end-pos > 255:
        raise ValueError("Invalid bounded EXR header")
    result = header[pos:end].decode("ascii")
    pos = end+1
    return result
while pos < len(header):
    name = text()
    if not name:
        break
    kind = text()
    count = struct.unpack_from("<I", header, pos)[0]
    pos += 4
    if pos+count > len(header):
        raise ValueError("EXR header exceeds 64 KiB")
    if name == "dataWindow":
        if kind != "box2i" or count != 16:
            raise ValueError("Invalid data window")
        x0, y0, x1, y1 = struct.unpack_from("<4i", header, pos)
        if x0 != 0 or y0 != 0:
            raise ValueError("Nonzero data-window offsets need a preservation-capable adapter")
        dimensions = (x1+1, y1+1)
    pos += count
if dimensions is None or min(dimensions) < 1 or dimensions[0]*dimensions[1] > cfg["max_pixels"]:
    raise ValueError("Invalid or over-budget dimensions")

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 1
scene.cycles.use_denoising = False
if hasattr(scene.render, "compositor_device"):
    scene.render.compositor_device = "CPU"
scene.render.threads_mode = "FIXED"
scene.render.threads = cfg["threads"]
scene.render.resolution_x, scene.render.resolution_y = dimensions
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = "OPEN_EXR"
scene.render.image_settings.color_mode = "RGBA"
scene.render.image_settings.color_depth = "32"
scene.render.image_settings.exr_codec = "ZIP"
scene.view_settings.view_transform = "Raw"
scene.view_settings.exposure = 0
scene.view_settings.gamma = 1
camera = bpy.data.objects.new("Denoise-only camera", bpy.data.cameras.new("Denoise-only camera"))
scene.collection.objects.link(camera)
scene.camera = camera
scene.use_nodes = True
nodes, links = scene.node_tree.nodes, scene.node_tree.links
nodes.clear()
image = bpy.data.images.load(str(source), check_existing=False)
reader = nodes.new("CompositorNodeImage")
reader.image = image
reader.layer = cfg["passes"]["layer"]
def output(name):
    for socket in reader.outputs:
        if socket.name == name:
            return socket
    raise ValueError("Missing actual EXR pass: " + name)
roles = [cfg["passes"][key] for key in ("beauty", "albedo", "normal")]
if len(set(roles)) != 3:
    raise ValueError("Beauty/albedo/normal must refer to distinct actual passes")
for role, expected in (("beauty", "RGBA"), ("albedo", "RGBA"), ("normal", "VECTOR")):
    if output(cfg["passes"][role]).type != expected:
        raise ValueError("Wrong socket type for " + role)
denoise = nodes.new("CompositorNodeDenoise")
denoise.prefilter = "ACCURATE"
links.new(output(cfg["passes"]["beauty"]), denoise.inputs["Image"])
links.new(output(cfg["passes"]["albedo"]), denoise.inputs["Albedo"])
links.new(output(cfg["passes"]["normal"]), denoise.inputs["Normal"])
alpha = nodes.new("CompositorNodeSetAlpha")
alpha.mode = "REPLACE_ALPHA"
links.new(denoise.outputs["Image"], alpha.inputs["Image"])
links.new(output("Alpha"), alpha.inputs["Alpha"])
composite = nodes.new("CompositorNodeComposite")
links.new(alpha.outputs["Image"], composite.inputs["Image"])
scene.render.use_compositing = True
scene.render.filepath = str(target)
bpy.ops.render.render(write_still=True)

# Decode the resulting flat float EXR. Do not claim alpha support based on flags.
decoded = bpy.data.images.load(str(target), check_existing=False)
if tuple(decoded.size) != dimensions or decoded.channels != 4:
    raise ValueError("Decoded output dimensions/channels mismatch")
pixels = decoded.pixels
for start in range(0, len(pixels), 16384):
    values = pixels[start:start+16384]
    if any(not math.isfinite(v) for v in values):
        raise ValueError("Non-finite output samples")
    if any(abs(values[k]-1.0) > 1e-6 for k in range(3, len(values), 4)):
        raise ValueError("Transparent input is outside the verified opaque-only contract")
print(json.dumps({"backend":"blender-oidn","blender_version":bpy.app.version_string,"oidn_version":"not_independently_reported","pixel_decode":True,"alpha_opaque_verified":True,"output":"linear float32 RGBA EXR beauty","original_passes_retained_in_source":True}))
