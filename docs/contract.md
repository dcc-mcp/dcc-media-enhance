# Agent contract, version 1

## Request

Required: schema version; `operation=denoise|upscale|resize`; `media=image|video`;
`content=render|photo|illustration`; input/output; backend or explicit `auto`;
color interpretation; opaque-alpha assertion; **all resource budgets**.
Unknown fields and unsupported combinations are rejected, not ignored. Limits
are mandatory because a scheduler/agent must decide what this machine can afford.

`auto` is a deterministic route table, not an online service or opportunistic
model change. It never falls from neural upscale back to interpolation, from CPU
to GPU, from EXR to an 8-bit display image, or from temporal to spatial processing
without a new request. There is no arbitrary shell-command escape hatch.

Render inputs require exact layer and beauty/albedo/normal socket names from the
real EXR. No synthesized auxiliary maps. Normal textures are not interchangeable
with render normal passes. Noisy AOVs use Blender's ACCURATE prefilter. The original
EXR is the source of truth. Current output is finite float32, opaque RGBA beauty;
all additional original channels remain in the unchanged source, not in output.
Only zero-offset matching data/display windows are accepted. Multipart/deep EXR,
non-square pixel aspect, and transparency require a future preservation adapter.
Initial multilayer EXR probing is explicitly `header_only`; output validation is
actual pixel decoding. Do not turn a header probe into a quality claim.

Non-render jobs currently accept declared 8-bit SDR interpretation only. There is
no ICC/OCIO/RAW development pipeline. Do not strip an HDR tag or lie about alpha to
make an unsupported input pass. Use an explicitly audited conversion workflow
outside this tool. No original metadata is modified.

## Lifecycle and cache

1. Validate request, source identity, layout and capabilities.
2. Plan a deterministic backend and record its runtime/model identities.
3. Acquire output OS lock, create private staging and persistent running state.
4. Execute a bounded subprocess; SIGINT/SIGTERM or `cancel` requests cancellation.
5. Decode output; verify dimensions/frame count, finite render pixels, source hash.
6. For video, compare presentation timestamps relative to the first video frame,
   audio packet SHA-256 and relative audio timestamps, tolerating 1.1 ms container
   quantization. Origin is normalized. This is not motion-quality validation.
7. fsync and publish no-clobber output; publish manifest as final commit marker.

Each output has `.manifest.json`, `.job.json`, and a persistent `.lock-owner`
sidecar. The lock file is not itself an active lock; the OS flock is authoritative.
`status` checks supervisor start identity, not PID existence alone. `cancel` writes
an idempotent owner-tagged sentinel rather than signaling an untrusted stored PID.
No routine command deletes an existing output or foreign sidecar. Cancellations
remove only the current worker's uncommitted pixel output. Bounded diagnostics
remain for diagnosis; there is no automatic history retention/deletion policy.

A completed manifest and current output hash, not file existence, prove a cache
hit. Source bytes, effective parameters including budgets, binary, linked shared
libraries, loader environment digest, model files, embedded helper and runner
identity participate in the key. Dynamically loaded plugins/custom color configs
are not supported cache identities in this MVP. Only use trusted installed
backends. Resume is at completed-job/enhancement-stage granularity, not partial
video-frame or model-tile granularity.

A crash between output and manifest publication is reported as interrupted or
publication-incomplete. Inspect the recorded staging/manifest candidate. Recovery
never trusts an uncommitted output, rewrites the original, or silently overwrites
that orphan output. Choose a fresh output name to retry if necessary.

## Limits and error semantics

All request fields ending in `_mb` use MiB (1024x1024 bytes), while RSS receipts
use KiB. Reserve extra capacity for the CLI supervisor and surrounding workload;
per-backend limits are not an inclusive whole-machine allocation.

CPU-only, one job at a time. Explicit thread hints are passed to backends. A
50-ms process-group RSS poll, wall timeout, staged output limit, bounded log and
disk reserve can stop the owned process group. Watchdogs may overshoot between
samples; they are not cgroup enforcement. Parent-death signaling protects the
direct worker; it is not a general untrusted-code sandbox. Backend tools must be
trusted and already installed. No network inference/download/provisioning occurs.

Budget timeouts apply to each subprocess, not the entire batch. Inspection has a
fixed 15-second, 256-MiB process limit and 2-MiB captured-result cap. Verification
of long videos currently fails at those bounds; do not describe this as unlimited
streaming. Model files are bounded and fingerprinted, not granted a blanket
redistribution license by the tool. A job is not guaranteed to fit just because
its header fits a pixel budget.

Output stdout is one JSON object. Error codes are stable categories with a human
message. Batch errors are per-item and produce nonzero process status. No result
claims a reference metric, visual review, semantic detail restoration, absence of
hallucinated details, no temporal flicker, or performance that was not measured.

## Future adapters, without duplication

- Standalone OIDN CPU worker via official CLI or Rust C-API bindings with audited
  EXR I/O. Keep native runtime out of the calling DCC process.
- Optional audited CPU super-resolution package. Keep `upscale` distinct from
  `resize`. Gate on model provenance, license, runtime and actual CPU regression.
- Streaming timestamp/audio verification and bounded temporal chunking with
  correct overlap before claiming long-video/segment resume.
- Existing scheduler integration via portable job/batch JSON. No custom cluster
  management service, automatic remote-machine startup, or hidden uploads.

Cancellation remains responsive during content hashing and is checked immediately
before publication. Once state is `committing`, new cancellation requests are not
accepted; `cancel_requested` means a request was queued, not proof of a cancelled
terminal state. Always read final status. The final publication boundary can race
with a request already in flight. Audio verification includes codec extradata and
packet side data as well as packet bytes and relative PTS; frame end durations and
subjective A/V/temporal quality remain outside the initial automated checks.

Liveness is scoped to the supervisor's Linux PID namespace. When a scheduler or
sandbox invokes `status` from a different namespace, it reports liveness as
unobservable rather than falsely declaring the job dead. The filesystem sentinel
still supports cancellation across those namespaces when the job directory is
shared. A scheduler remains responsible for reconciling a terminated container.

Cluster portability is a job-data boundary, not a tested distributed locking or
cross-machine bit-exact cache guarantee. Use isolated, authorized input/output
locations and a scheduler-compatible filesystem. Local no-clobber links/flocks
must be validated on any proposed shared filesystem before concurrent deployment.

## Reviewed recovery boundaries

Cache reuse requires a complete versioned success manifest: request and resolved
source/output paths, source/backend/runtime/model identities, output hash, resource
receipt, and all required verification/provenance fields must match. A copied job
folder is not an automatic cross-location cache migration. Missing or altered
manifest fields cause a no-clobber conflict, never a partial success response.

Existing sidecars are checked with symlink metadata, including dangling links.
Foreign links are rejected unchanged. Cancellation sentinels propagate through
all output verification probes; a cancelled batch does not continue later jobs.

Source identity covers a single self-contained file. FFmpeg and FFprobe accept
only explicit raster-pipe and leaf video demuxers, with file-only input protocols.
Playlists, image sequences, concat/HLS/DASH and implicit network inputs are rejected
even if renamed to an image extension. MOV external tracks and absolute data
references are explicitly disabled. Convert referenced inputs in a separate,
authorized ingestion step before submitting a self-contained file. See FFmpeg's
[format options](https://ffmpeg.org/ffmpeg-formats.html) and
[protocol controls](https://ffmpeg.org/ffmpeg-protocols.html).
