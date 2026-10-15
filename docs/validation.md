# Historical local validation report (0.1 preview)

Initial tests: 2026-10-12. Consumer follow-up: 2026-10-13. Linux x86_64 CPU cloud host. This is a local preview build,
not a published release or a quality/performance benchmark.

This dated report describes the original 0.1 Linux preview, not the current
0.2 native matrix. For current OS-specific CI execution and distribution limits,
see [the native platform contract](platforms.md) and the checks for the exact
commit being used. The measurements below are retained as historical evidence;
they are not relabeled as tests of later binaries.

## Fixed tested CLI

- Version: 0.1.0; Rust 1.95.0 build toolchain, Linux x86_64.
- Binary bytes: 968824.
- ELF inspection reports required GLIBC symbols through **2.39** and dynamic
  `libgcc_s.so.1`. This preview is not a manylinux/older-distro portable build.
- SHA-256: `7e3a69c2966622b2a24776039f955e52d670fe06bb7d08763f7663ce9a1aa95a`.
- Official source dependencies are pinned in Cargo.lock; no new Rust toolchain
  was installed for this task. Final temporary build cache was about 56 MiB.
- No GPU inference, global backend installation, model training, new MCP server,
  cluster provisioning or scheduler was used.

## Successful tests

- 10 optimized Rust unit tests: hash identity, invalid budgets, malformed EXR,
  pass-role distinctness, file-lock exclusion/recovery, timeout, cancellation,
  final-hash cancellation, and cleanup of a background grandchild after its
  leader exits.
- 9 real FFmpeg/CLI regression groups, all passed in 37.064 seconds total:
  discovery/schema/plan; image denoise + verified cache; fail-closed contracts;
  symlink/foreign sidecars; resize distinct from neural SR; VFR video and audio;
  partial batch resume; dead-supervisor status/idempotent cancel; live CLI cancel
  followed by successful retry against the retained original.
- Independent Blender 4.5.14 LTS EXR test through this same final binary. Real
  Combined / Denoising Albedo / Denoising Normal passes, ACCURATE prefilter,
  finite opaque float32 RGBA linear EXR, full output decode, original SHA unchanged.
- FFmpeg 7.1.5 Debian GPL-enabled build. The FFmpeg test fixtures were original
  synthetic images/video, not downloaded sample art.

Backend-stage observations (not whole CLI latency; sampled RSS is not a hard peak):

| Test | Backend ms | Sampled peak RSS KiB |
|---|---:|---:|
| image denoise, 32×24 | 327 | 27360 |
| VFR video denoise, 32×24 / 10 frames + audio | 511 | 33304 |
| Lanczos resize, 32×24 → 64×48 | 479 | 30024 |
| cancelled-job retry, 512×512 | 455 | 38808 |
| render denoise, 128×86 EXR | 2475 | 287588 |

Video validation matched frame count, relative presentation timestamps within
1.1 ms, audio codec sequence, codec extradata, packet side data, packet SHA-256,
and relative audio timestamps. Absolute time origin is normalized. Frame-end
intervals, subjective lip-sync/ghosting, large video and HDR/alpha are not claimed.

The final EXR test succeeded with a 512-MiB watchdog. Another attempt at 384 MiB
was stopped by the memory watchdog, left no committed output, and retained the
original source hash. The earlier successful tiny test sampled about 281 MiB.
This variation is why no universal resolution-to-RAM promise is made. The supplied
preview-render template requests 1024 MiB and one thread, not the smallest
observed number. Larger inputs still require real capacity validation.

## Real render consumer follow-up

The fixed CLI above was subsequently used by a real render consumer, after the
renderer exited: retained raw EXR -> separate CLI denoise -> separate AgX PNG
encoding. Three 640x480 frames completed with real ViewLayer Combined / Denoising
Albedo / Denoising Normal passes. Their manifests record full pixel decode,
scene-linear float32 RGBA, unchanged sources, and matching output hashes. Sampled
backend RSS was 515940-525164 KiB; backend time was 24.692-36.212 seconds. This is
one actual consumer integration, not migration of every DCC host. Original scenes,
raw files and complete machine-specific manifests are not redistributed here.

A subsequent 1280x960 consumer exposed the importance of explicit capacity
planning. One frame completed under 1152 MiB, but a different frame returned
`MEMORY_LIMIT` under both 1152 and 1200 MiB. Retained raw input allowed enhancement
alone to be retried. With an explicitly replanned **1450-MiB backend budget** and
**1536-MiB inclusive outer job budget**, that frame completed in 54.332 seconds
with sampled backend RSS 1247268 KiB. The outer supervisor separately observed
1255916 KiB inclusive RSS. Another completed frame sampled 1281068 KiB backend
RSS. These independently sampled numbers are not exact simultaneous peaks.

All schema `*_mb` fields use **MiB (1024x1024 bytes)**; RSS receipts use **KiB**.
A per-backend budget does not include the calling supervisor or every unrelated
process. Leave scheduler/container headroom and account for concurrent workloads.
No global/default budget was raised as a result of these observations. Do not
retry a resource failure indefinitely or silently increase its authorized limit;
re-plan explicitly, preserve the original, and retry only the enhancement stage.

## Neural CPU super-resolution: unavailable on this host

Both official waifu2x 20250915 Linux and 20250504 Ubuntu CLIs failed actual CPU
inference with explicit `-g -1`; no output was accepted. The 20250915 diagnostic
confirmed SIGSEGV following `vkCreateInstance failed -9`. The older CLI also
terminated abnormally. The model pair was byte-identical across both releases.
The exact crash root cause is **not established**. No driver was installed and
no sandbox/ptrace permission was expanded. Testing stopped after the bounded
fallback to the older official package.

The 20250915 archive SHA matched the publisher's API digest. The older asset had
no published digest; its HTTPS origin and locally computed SHA are recorded,
not mislabeled as independent checksum verification. Only executable/license and
one photo model pair were locally extracted; none are redistributed in this source
tree. Reproducible ZIP download caches were removed after recording hashes to
restore disk headroom. The [upstream reproduction draft](upstream-cpu-repro-draft.md)
is **unsent**. Lanczos resize remains separately named and is not substituted for SR.

## Remaining boundaries

The original 0.1 preview had no Windows/macOS CI; later native CLI/FFmpeg gates
are documented separately in [the 0.2 platform contract](platforms.md). No
standalone Rust/OIDN worker, released offline backend bundle, transparent/HDR-video
support, streaming long-video verification, neural-quality benchmark or blanket
DCC-host migration is claimed by this report. Visual/reference quality
checks are `not_run` in manifests. The source repository, CLI, skill and job contract are preview work for review.
No release, blanket host migration, or production-readiness claim is implied by
this validation report.
