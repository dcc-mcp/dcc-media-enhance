# Unsent upstream issue draft

This draft has **not** been sent or published to the upstream project.

## Suggested title

CPU `-g -1` exits abnormally on a Linux x86_64 headless host with a 32×32 RGB PNG

## Environment and scope

- Linux x86_64 cloud host; installed glibc, libstdc++, libgomp and Vulkan loader.
- No GPU/ICD driver is available in the inspected standard Vulkan ICD directories.
- No driver was installed and no system configuration was changed during testing.
- Official prebuilt waifu2x packages, with the supplied `models-upconv_7_photo`
  `scale2.0x_model.bin/.param` pair, no source patches.
- Input is an original synthetic opaque 8-bit RGB PNG, not RGBA or a user photo.
- CPU-only flags: `-g -1 -n -1 -s 2 -t 32 -j 1:1:1`, `OMP_NUM_THREADS=1`.

## Reproduction

Create a synthetic image using installed FFmpeg, or supply any equivalent 32×32
opaque RGB PNG. No private production media is needed.

```sh
python3 - <<'PY'
from pathlib import Path
Path('source.ppm').write_bytes(b'P6\n32 32\n255\n' + bytes([77,122,189]) * (32*32))
PY
ffmpeg -v error -nostdin -n -threads 1 -i source.ppm -threads 1 source.png
OMP_NUM_THREADS=1 ./waifu2x-ncnn-vulkan \
  -i source.png -o output.png -g -1 -n -1 -s 2 -t 32 -j 1:1:1 \
  -m models-upconv_7_photo
```

Expected: a 64×64 neural-upscaled PNG using the documented CPU path.
Observed with **20250915**: `vkCreateInstance failed -9` followed by abnormal
termination. A bounded direct subprocess returned **-11 (SIGSEGV)** and no output.
The initial orchestrated run used an RSS watchdog, not an address-space limit;
it also failed. A 32×24 RGB fixture and the square fixture both failed.

The official **20250504** CLI was also tested, reusing model files confirmed
byte-for-byte identical to those in its release archive. It exited abnormally and
created no output. Its exact termination signal was not independently captured.
The two packages' CPU failures do not prove that Vulkan initialization is the root
cause. A syscall trace could not be obtained because this sandbox denies ptrace;
no privilege/sandbox change was attempted.

## Artifact identities

20250915 source:
https://github.com/nihui/waifu2x-ncnn-vulkan/releases/tag/20250915

- Linux archive SHA-256, **matched to the publisher's release API digest**:
  `848e0fba55657d34da90b775b8139e9806dc754798b029f95e106ba8850a731f`
- Executable SHA-256:
  `c18f4ec21879a8b91263551ebaab0a5b5f45ad4c456d90cf626aaaf561b42280`

20250504 source:
https://github.com/nihui/waifu2x-ncnn-vulkan/releases/tag/20250504

- Ubuntu archive SHA-256, **locally computed only**; release API digest was null:
  `4c60981b75b8e89c36bd968de9e4b34d1c33054eadcf8a3fcee614ad9fe9311b`
- Executable SHA-256:
  `54f81896117d5abb1e3bb83c35fb7773156e9f5ee9b1ef8a182587e2ee8c83a8`

Identical model pair in both archives:

- `scale2.0x_model.bin`:
  `25a2bb25b29e43e63179aac216cd87690791243a436328506d4ef9fca88ee962`
- `scale2.0x_model.param`:
  `413195ffde05b4d43807792c6c020c916cecdf25dcf002ee83f5e28d5cc246c6`

Is there a supported CPU-only configuration for this headless environment, or a
prebuilt CPU-only artifact that avoids GPU runtime initialization? We have not
assumed that the similar-looking older issue #186 is the same root cause.
