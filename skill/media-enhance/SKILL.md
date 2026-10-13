---
name: media-enhance
description: Denoise retained render EXRs, SDR images or videos with explicit CPU budgets and provenance using the dcc-media-enhance CLI. Also distinguish deterministic resize from optional neural upscale. Use for post-render denoising, agent batch processing, or planning machine-compatible enhancement.
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
Default to CPU and serial jobs. The sampled RSS watchdog is not a hard OS limit;
use the existing scheduler/container for hard isolation. No cluster is started.

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
