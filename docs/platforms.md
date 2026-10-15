# Native platform contract (0.2)

CI executes each release binary on its own OS and architecture. A compiled binary
alone is never counted as a runtime pass. The matrix is Linux x86_64 (Ubuntu
24.04), Windows x86_64 (Server 2022), macOS x86_64 and macOS ARM64 (macOS 15).
No Windows ARM64, Linux ARM64, universal macOS binary, old-glibc compatibility,
macOS notarization, or Windows signing is claimed. Windows builds statically link
the MSVC CRT and packaging rejects an external VCRUNTIME/MSVCP/UCRT DLL dependency;
OS-provided Windows system DLLs remain required. No redistributable installer is run.

| Boundary | Linux | Windows | macOS |
|---|---|---|---|
| Owned descendants | process group | Job Object assigned while child is suspended | process group |
| Cancel / timeout | owned group signal | terminate owned Job | owned group signal |
| Memory | sampled group RSS | aggregate Job committed-memory hard limit plus sampled RSS | sampled group RSS using libproc |
| Lock | no-follow file + flock | no-reparse handle + deny write/delete sharing | no-follow file + flock |
| Identity | PID namespace + process start ticks | process creation FILETIME | libproc start timestamp |
| Free space | statvfs | GetDiskFreeSpaceExW | statvfs |
| Sidecar replacement | rename + directory fsync | MoveFileExW replace/write-through | rename + directory fsync |
| Verified cache reuse | supported within documented loader scope | disabled | disabled |

Windows/macOS loader closure is not yet completely fingerprinted. These platforms
record the backend binary/version (and Windows colocated DLLs), but refuse to reuse
an existing output as a cache hit. They do not weaken cache validation or overwrite
that output. The agent must skip already completed outputs explicitly when
resubmitting a batch. Linux keeps its original runtime hash and cache behavior.

macOS has no equivalent of Linux's direct-child parent-death signal in this
implementation. Cooperative cancellation and normal supervisor cleanup kill the
owned group; an abruptly SIGKILLed supervisor cannot promise cleanup. Use an
external job supervisor for that guarantee. Windows Job handles close on process
exit and terminate their assigned descendants. These are trusted-backend resource
controls, not a sandbox against malicious process escape.

Windows rejects alternate data streams, reserved device names and ambiguous
trailing dots/spaces. Staging inherits its parent ACL; callers should select a
private output directory. NTFS/local filesystems with hard-link support are
required for no-clobber publication. Network shares and cross-machine lock
semantics are not validated. Unix staging mode is 0700.

CI runs native process-tree, memory-observation, cancellation, locking, Unicode
and space-path tests; real FFmpeg image/video/resize and source-retention tests;
and the Core skill bridge. CI does not install Blender or neural models,
so OIDN/render and neural-upscale quality on Windows/macOS remain unverified.
The historical Linux render test does not establish those other platforms.

## CI / distribution

Each main/PR build creates four independently tested zip artifacts for seven days.
Archives contain the CLI, schemas, examples, canonical skill, build metadata, pinned
crate notices and the exact Rust toolchain notices. They contain no FFmpeg,
Blender, weights, Microsoft toolchain files or user media. Inner SHA256SUMS covers
archive files; the adjacent .sha256 covers the zip. Build metadata carries target,
source commit, version, rustc details and the exact validation scope.

Windows verifies msvc-kit v0.2.22 SHA-256
`184f07fb2896f0a7d27c45445990bd7d9e525b6ee58956cfe857bf0027848925`
before execution. It inspects the runner's registered MSVC/SDK through a job-local
junction view, runs the real doctor compile/resource/link/execute probes, and
builds/tests inside `msvc-kit run`. Nothing silently substitutes a system VS
check for that doctor; no global PATH, registry, SDK install or credentials change.

Publishing a versioned GitHub Release is a separate maintainer action. After that
explicit event, release.yml reruns native gates and attaches all four archives
and their aggregate checksums without --clobber. It creates no tag or release and
never changes the repository's visibility. A tag/version mismatch or a
failed platform gate prevents attachment. No release has been created merely by
adding these workflows.
