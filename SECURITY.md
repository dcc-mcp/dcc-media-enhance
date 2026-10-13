# Execution boundary

This CLI orchestrates **trusted, already-installed native backends**. It is not a
sandbox for untrusted executables or untrusted shared libraries. It never executes
a shell command supplied by a JSON job. PATH selection skips relative directories;
resolved executables, linked libraries and selected model files are fingerprinted.
Do not add an untrusted directory to PATH/loader paths or replace a backend while
processing. Factory-startup and disabled autoexec isolate Blender from scene code;
the adapter opens a media EXR, not an arbitrary `.blend` project.

Only run jobs whose source/output paths are authorized for the caller. Results and
manifests can contain local paths and source identities; review them before sharing.
There is no telemetry, model downloader, credential handling, or remote scheduler.

Resource watchdogs are best-effort observations and can overshoot. Use OS cgroups,
container quotas or your existing scheduler when hard isolation is required.
The Linux supervisor cleans the owned process group before reaping its leader,
including when the leader exits before a background child. A backend that escapes
its process group, or the supervisor being force-killed, requires OS-level isolation
for strong descendant/resource guarantees. Only a committed, matching manifest
proves an accepted output; incomplete publication must be inspected, not trusted.

The optional waifu2x test package is obtained from its official release, checksum
verified and locally extracted. It is not included in the source distribution.
Any future bundled backend/weight needs an independent supply-chain/license audit.

All input probes/decode commands restrict demuxers and input protocols. This
single-file contract intentionally excludes playlists and referenced-file
sequences; filenames alone do not select trusted media formats. Cache manifests
are local provenance records, not authenticated signatures against a malicious
writer. Use caller-authorized private directories; a party able to rewrite source
files, backend binaries, directory entries or manifests is outside this CLI's
isolation boundary. No-clobber output does not claim adversarial shared-directory
TOCTOU protection.
