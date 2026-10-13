# dcc-media-enhance

A small, agent-first **orchestration CLI**, implemented in Rust. It calls mature
installed media tools rather than reimplementing denoising algorithms or training
another model. Jobs have explicit resource budgets, machine-readable results,
source provenance, verified outputs, and resumable serial batches.

**Initial target: Linux CPU cloud machines.** GPU, another MCP server, a DCC host,
a network connection, and a cluster scheduler are not required by the CLI. Individual
backends retain their own requirements. Render denoising currently uses an installed
Blender; standalone OIDN is a planned replacement/alternative, not shipped yet.

## Operations and boundaries

| Operation | Backend | Initial support |
|---|---|---|
| Render denoise | Installed Blender compositor / OIDN | Opaque, flat single-part scene-linear EXR with real beauty, albedo and normal passes; float32 RGBA beauty output |
| Image denoise | Installed FFmpeg `nlmeans` | Opaque 8-bit SDR image to PNG |
| Video denoise | Installed FFmpeg `atadenoise` | Opaque 8-bit SDR to lossless FFV1/Matroska; copied audio; independently checked relative timestamps |
| Resize | Installed FFmpeg Lanczos | Deterministic CPU 2× interpolation; **not neural super-resolution** |
| Neural upscale | Optional `waifu2x-ncnn-vulkan -g -1` | Experimental adapter only; actual CPU probes failed on this cloud for official 20250915 and 20250504 releases; unavailable here |

No backend or weight is automatically downloaded. Unsupported HDR video,
transparency, rotation, non-square pixels, extra subtitle/data streams, or unknown
formats fail closed. Render EXR does not get tone-mapped or silently converted to
PNG. Source passes and metadata remain in the retained original EXR; this first
adapter produces a separate beauty master rather than claiming to preserve every
channel in the derivative.

## Build and use

Rust 1.85+; Linux. Dependencies are pinned in `Cargo.lock`.

```sh
CARGO_BUILD_JOBS=1 cargo build --locked
./target/debug/dcc-media-enhance doctor
./target/debug/dcc-media-enhance schema
./target/debug/dcc-media-enhance probe source.png
./target/debug/dcc-media-enhance plan examples/image.json
./target/debug/dcc-media-enhance run examples/image.json
./target/debug/dcc-media-enhance batch examples/batch.json
./target/debug/dcc-media-enhance status output.png.job.json
./target/debug/dcc-media-enhance cancel output.png.job.json
```

Edit example input/output paths before running. Paths inside jobs are relative to
the job JSON file, not the caller's working directory. Output parents must already
exist. Every response is a single JSON object on stdout; backend logs are separate,
bounded local artifacts. A backend cannot be replaced by an arbitrary command in a
job. `doctor` observes installed capabilities and available capacity; it does not
reserve capacity or install software.

- Exit 0: success, including verified cache hit
- Exit 2: invalid or unsupported request / no-clobber conflict
- Exit 3: dependency, backend, validation, I/O, or partial batch failure
- Exit 4: resource budget exceeded
- Exit 5: cancelled

See [the JSON schema](schemas/job.schema.json), [contract](docs/contract.md),
[backend selection and licenses](docs/backends.md), and
[agent skill](skill/media-enhance/SKILL.md).

## Reliability model

The original is hashed before and after processing and never opened for writing.
Workers write to private same-filesystem staging, then the output is decoded and
checked. Publication uses no-clobber hard links plus fsync. The final manifest is
the commit marker. Because two filenames cannot be committed in one filesystem
operation, a crash can leave a complete orphan output; it is never treated as a
cache hit, silently removed, or overwritten. Inspect the recorded state and use a
new output path if necessary.

A cache hit requires the same effective request, input content, executable, linked
runtime libraries, model files, helper/runner code and a matching output hash.
OS file locks exclude concurrent writers and are released after crashes. Retrying
a batch reuses verified completed items; failed items restart their enhancement
stage against the preserved source. Interrupted staging is not assumed valid.

Thread hints, pixel/byte limits, disk reserve, timeout and a sampled process-group
RSS watchdog are enforced. **The RSS watchdog is not an OS hard memory boundary.**
Use an existing cgroup/container/scheduler for hard isolation. Budgets apply per
subprocess; probes additionally have a 15-second/256-MiB/2-MiB result cap. Video
verification is intentionally bounded and rejects an oversized probe response.
Long videos will need a future streaming verification implementation.

## DCC-MCP integration

The canonical skill lives in this repository. Existing host skills should save a
raw EXR with true auxiliary passes, release the render process, then invoke this
CLI as a separate job. Denoise failure must not trigger a re-render. Linear EXR is
the master; display transforms and preview encodes are explicit later stages.

This is the preferred contract for future rendering workflows once the tool is
installed and verified in that environment. Existing Blender/Houdini/other host
routes have **not** all been migrated. The public marketplace should reference
this skill at an immutable revision after review, rather than duplicate its body.
Existing Houdini `idenoise` and texture/OCIO tools remain complementary adapters.

Batch JSON is portable input for an existing scheduler. The scheduler provides
asset placement, machine selection, hard limits and retries; this project does not
provision machines, manage a cluster, or invent another queue service.

## Verification and distribution status

See the [measured local validation report](docs/validation.md).

Run `cargo test --locked` and the actual-backend
[smoke tests](tests/README.md). Decode success and synthetic regression tests are
not artistic quality benchmarks. Manifests distinguish executed checks from
`not_run`; enhanced derivatives are not evidence that previously unreadable detail
was factual.

This source tree is an initial local implementation with a locally compiled Linux preview binary. It has no published release,
no prebundled third-party runtime/model, and no claim of Windows/macOS support.
The local preview requires glibc 2.39+ and libgcc_s; older-distro distribution
needs a separately built/tested baseline artifact. A Rust executable does not
remove backend ABI, native-library, model-license or
supply-chain obligations. Rust dependency notices are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
The wrapper's MIT license does not relicense FFmpeg,
Blender, OIDN, ncnn, or model weights.
