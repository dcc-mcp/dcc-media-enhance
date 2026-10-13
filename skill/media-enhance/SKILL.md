---
name: media-enhance
description: Denoise retained render EXRs, SDR images or videos with explicit CPU budgets and provenance using the dcc-media-enhance CLI. Also distinguish deterministic resize from optional neural upscale. Use for post-render denoising, agent batch processing, or planning machine-compatible enhancement.
license: MIT
compatibility: Python 3.12+ for Core bridge; explicitly installed native CLI and media backends
metadata:
  dcc-mcp:
    dcc: python
    version: "0.2.0"
    layer: infrastructure
    search-hint: "media enhancement, CPU denoise, render EXR, image video, resize, bounded batch"
    tags: "media, rendering, denoise, postprocess, cpu"
    tools: tools.yaml
---

# Media enhancement

Use the installed `dcc-media-enhance` CLI as the common enhancement boundary.
First run `doctor`, inspect the source, read `schema`, then create a versioned job
and call `plan`. Use only capabilities actually reported by this environment.
Never install a backend, download a weight, provision a machine or upload media
as an implicit part of processing.

## Render workflow

For future render jobs in environments with this tool installed and verified:

1. Save the original scene-linear EXR with true beauty, denoising normal and
   denoising albedo passes before requesting enhancement.
2. Finish/exit the render worker so geometry memory is released.
3. Call this tool in a separate bounded CPU job with exact layer/pass mappings.
4. Accept only a completed, verified manifest and matching output hash. Keep the
   raw EXR. If denoise fails, retry denoise against that EXR rather than re-render.
5. Keep the resulting linear EXR master separate from explicit display-transform
   and preview encoding stages.

Existing host workflows are not automatically migrated merely because this skill
exists. When the tool/backend is missing, report that exact blocker and preserve
raw output. Do not silently claim universal host integration or fall back to a
scene-bound denoiser without making the selected path explicit.

## Budget and routing

Use explicit threads, memory, timeout, pixel/byte limits and free-disk reserve.
All schema `*_mb` fields mean MiB (1024x1024 bytes); RSS receipts are KiB.
Budget the outer supervisor and runtime headroom separately from the backend.
Consider other active workloads; `doctor` observations are not reservations.
A previous frame fitting is not proof the next will fit. On resource failure,
retain raw, re-plan against actual available capacity, and retry enhancement only.
Do not silently raise limits or change global defaults to force a job through.
Default to CPU and serial jobs. Read `memory_enforcement` for the native policy.
Windows also caps Job Object commit memory; Unix RSS watchdogs are not hard limits.
Use the existing scheduler/container for whole-job isolation. No cluster is started.

Use render models only for render noise. Photos and video have separate routes.
Noisy render AOVs need the selected backend's proper prefilter; never invent AOVs.
Alpha, HDR, color conversion, non-square pixels, rotation and unsupported streams
must not be coerced to fit this initial opaque SDR / restricted EXR contract.
A missing backend is a typed failure, not permission to silently change methods.

`resize` is interpolation. `upscale` means an explicitly available, audited neural
backend and model. Experimental CPU upscale is not counted as verified until the
installed package passes a real test. Video temporal averaging may ghost; spatial
or per-frame neural methods can flicker.

## Execution and acceptance

Call `run` with the saved JSON job. Use `status` and `cancel` for the owned job.
For batches, use versioned batch JSON; rerun to reuse completed verified outputs.
Do not reuse interrupted staging as a success result or overwrite existing files.

Read the manifest's source hash, backend/runtime/model identities, measured
resource receipt, warnings and individual verification fields. A decoder passing
is not a visual-quality review. Mark unsupported or unperformed checks honestly.
Enhanced outputs are derivatives and are not proof of unreadable text or hidden
scene details. Share originals alongside enhanced deliverables when facts matter.

Contract and backend details are maintained in `docs/contract.md` and
`docs/backends.md`; the canonical request schema is `schemas/job.schema.json`.

On the initial dot cloud validation host, official waifu2x releases 20250915 and
20250504 failed real CPU inference. Treat neural upscale as unavailable there;
do not substitute Lanczos and call it neural SR. Revalidation is required on a
different, explicitly provisioned runtime.

## Existing dcc-mcp-cli integration

Expose this private skill through an explicitly configured `DCC_MCP_SKILL_PATHS`
root in the existing Python host. Configure `DCC_MEDIA_ENHANCE_BIN` as an absolute
installed binary path and `DCC_MEDIA_ENHANCE_SHA256` to its verified digest in that
host's environment. This repository never changes the global PATH or registers a
server by itself. Search through the existing `dcc-mcp-cli search`, then follow
its returned `load-skill`, `describe`, and `call` instructions. Do not guess a
server ID or duplicate Core's MCP/REST transport.

Tools use Core's `skill_success` / `skill_error` envelopes and retain the entire
CLI result under `context.media`. Long execution uses Core's existing async-task
metadata; media `status` / `cancel` continue to reference the CLI state path.
Outer Core-task cancellation is distinct from a confirmed media terminal state.
The bridge has an explicit 900-second default outer budget; choose it deliberately
for longer jobs. Full loader fingerprints and cache reuse are currently Linux-only.
