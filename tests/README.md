# Verification

`python3 tests/smoke.py --binary /absolute/path/to/dcc-media-enhance`

Tests use actual installed CPU FFmpeg backends with original deterministic 32×24
fixtures, including VFR video and audio. Each run creates its own directory. They
require at least 700 MiB free disk, do not download anything, do not use a GPU,
and do not process private production scenes. They check source hashes,
no-clobber publication, verified cache hits, typed rejections, real decoding,
relative VFR timestamps, copied audio packet hashes, batch resume, and sidecar
ownership. Resize is tested as resize, never represented as neural super-resolution.

Blender/OIDN and waifu2x need separate installed-backend opt-in tests; the ordinary
smoke test must not silently install them or claim they ran. A passed pixel decoder
is not proof of artistic quality, natural detail recovery, or temporal consistency.

Opt-in tiny retained-EXR test (configure PATH to the intended installed Blender):

```sh
python3 tests/render_probe.py --binary /path/to/dcc-media-enhance \
  --input-exr /path/to/opaque-multilayer-128x86.exr
```

This helper caps the source at 8 MiB/16,384 pixels, verifies source SHA-256,
checks a decoded linear float32 RGBA EXR result, and retains its report. It does not
open a production scene or trigger a re-render. No source EXR is redistributed in
this repository.
