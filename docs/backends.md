# Backend decisions and licensing

Research snapshot: 2026-10-12. Upstream code, models and distribution artifacts
have separate identities. This repository contains only original adapter code;
no external backend implementation, binary, checkpoint or example art is copied.

## Render

[Open Image Denoise](https://www.openimagedenoise.org/documentation.html) RT is
for ray-traced Monte Carlo noise, not generic camera noise. Its official
[oidnDenoise CLI](https://github.com/RenderKit/oidn/blob/master/doc/examples.md)
already exists; image formats depend on build options (PFM baseline, EXR through
optional OpenImageIO). Reuse it or the existing
[oidn-rs bindings](https://github.com/Twinklebear/oidn-rs) rather than create a new
algorithm. A native runtime and its dependencies remain required. OIDN and the
[official weights](https://github.com/RenderKit/oidn-weights) use Apache-2.0;
Rust bindings use MIT. Verify the actual artifact and notices before bundling.

For the first installed-backend path, Blender's compositor denoiser is isolated
in an empty process reading an existing EXR. This avoids retaining a production
scene's geometry during denoising. Blender is GPL software. Subprocess invocation
is not an exemption from redistribution obligations; no Blender is bundled here.
Its actual OIDN version is not inferred from its Blender version.

## Images and video

[FFmpeg filter documentation](https://ffmpeg.org/ffmpeg-filters.html) documents
`nlmeans` and `atadenoise`. The former is spatial; the latter is temporal averaging,
not motion-compensated neural video restoration. Test ghosting, cuts, motion and
loss of detail before accepting a production output. `hqdn3d` is a valid optional
alternative but its filter source is GPL-2.0-or-later. `nlmeans` and `atadenoise`
source files are LGPL-2.1-or-later; the **whole FFmpeg build** can be GPL due to its
configuration/dependencies. See [FFmpeg legal guidance](https://ffmpeg.org/legal.html).
The current cloud FFmpeg is a GPL-enabled Debian build, not an LGPL-only bundle.

Later neural candidates include [NAFNet SIDD denoising](https://github.com/megvii-research/NAFNet)
and [FastDVDnet](https://github.com/m-tassano/fastdvdnet). NAFNet's repository is MIT
with separate BasicSR licensing; external weight redistribution was not established
by this review. FastDVDnet's MIT repository contains checkpoints but each bundled
weight must still have documented origin/license/hash. Its AWGN assumptions are
not a guarantee for arbitrary compressed phone footage. Neither candidate’s weights were installed,
converted, benchmarked or included. Optional ONNX Runtime adds a native runtime;
an export must pass numerical parity before being called a working backend.

## CPU-first super-resolution

[waifu2x-ncnn-vulkan release 20250915](https://github.com/nihui/waifu2x-ncnn-vulkan/releases/tag/20250915)
has a real CPU path (`-g -1`) in its CLI source. This is the first optional CPU 2×
SR candidate. The adapter uses that explicit CPU flag, `-n -1` for upscale-only,
small tiles, bounded threads and caller-supplied model files. The CLI is MIT, ncnn
is BSD-3-Clause; retain actual model origins and licenses as independent fields.
Do not bundle upstream sample art, some of which has noncommercial licensing.
The adapter remains experimental: actual CPU inference failed on this cloud with both official 20250915 and 20250504 Linux packages. The first produced a confirmed SIGSEGV after a Vulkan-initialization diagnostic; the older package also exited abnormally. No output was published. This does not establish the crash root cause. See the unsent reproduction draft and validation report.

[RealSR ncnn](https://github.com/nihui/realsr-ncnn-vulkan) is another CPU-capable
candidate, but its older larger release is not a first dependency. Its wrapper
license does not replace upstream RealSR/model provenance review.
[Real-ESRGAN ncnn v0.2.0](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan/tree/v0.2.0)
rejects negative GPU IDs; ncnn's CPU support alone does not make that CLI a CPU
backend. Keep it GPU-optional, not a promise for no-GPU cloud machines.

SR can synthesize plausible textures and lettering. Retain originals, label all
outputs as derivatives, and never interpret recovered-looking details as factual
evidence. Image models applied frame-by-frame do not guarantee temporal stability.

## Packaging gates

Start with a small Linux Rust CLI plus trusted installed backend executables.
Before distributing an optional backend bundle, record source URL and revision,
artifact SHA-256, model SHA-256, build flags, platform/CPU requirements, shared
runtime dependencies, software AND weight licenses, third-party notices, SBOM,
source-offer obligations where applicable, and actual regression evidence. No
single-executable/zero-runtime or cross-platform guarantee has been validated.
