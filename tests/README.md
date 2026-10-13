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

Reviewed failure-boundary regressions:

```sh
python3 tests/risk_smoke.py --binary /absolute/path/to/dcc-media-enhance
```

These use original 8x8 pixels and real installed FFmpeg for success/cache tests.
They reject incomplete manifests, relocated cache provenance, dangling foreign
sidecars and disguised ffconcat input. The cancellation test deliberately
interposes one local sleeping ffprobe fixture after real enhancement to prove
verification cancellation stops a batch. That fixture is not described as a real
probe or media-quality test. No renderer, models or network are used.

CI runs unit tests with `--release`, builds the optimized CLI with the locked
dependencies, and runs both suites against that same release-profile executable
before packaging it. The success path includes a completed manifest and verified
cache; the live cancellation path must preserve the source, publish no output, and
then complete a retry. A Cargo release profile is a build configuration, not a
published GitHub release. Installed-Blender EXR acceptance remains a separate gate.
