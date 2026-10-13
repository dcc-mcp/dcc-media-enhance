# Private dcc-mcp-cli integration

`dcc-mcp-cli` is shipped by `dcc-mcp/dcc-mcp-core`, not a second media server.
This adapter reuses Core's existing skill catalog, typed tools, SDK result
helpers, CLI search/describe/call path, async metadata and MCP Client transport.
It adds no network listener, alternate MCP protocol, global PATH or host registry
mutation. All media implementation and skill files stay in this private repo.
No private source needs to be copied into the public Core repository.

The checked SDK baseline is dcc-mcp-core 0.20.44. Compatibility stays within
`>=0.20.44,<0.21`; a later minor requires revalidation. CI pins the exact baseline
wheel and verifies native catalog parsing, skill discovery, standard envelopes,
binary-identity rejection and a real CLI run/status round trip on all four native
platforms. A separate Linux gate runs the actual dcc-mcp-cli 0.20.44 through an
isolated loopback gateway, discovers and loads the private Python skill, validates
wire schemas/annotations, and executes real FFmpeg resize plus explicit media
cancellation. This does not claim installation into an existing user gateway or
arbitrary DCC-host acceptance.

## Explicit host setup

In an already authorized Python skill host, add this repository's `skill` folder
to its process-local `DCC_MCP_SKILL_PATHS` (platform path separator applies).
Set `DCC_MEDIA_ENHANCE_BIN` to the absolute installed platform binary and
`DCC_MEDIA_ENHANCE_SHA256` to its verified archive-manifest digest. Restart/reload
that host through its existing supported workflow only when authorized. The
adapter cannot accept a caller-supplied executable, command shell or environment.

From the existing dcc-mcp-cli, search for media enhancement once and follow the
returned server-specific next step to load the skill, describe its exact tool
name and invoke it. Do not invent a server ID or assume the catalog prefix.
The skill exposes `inspect`, `execute`, `cancel` with explicit command enums,
absolute file paths and a bounded outer timeout. Requests/results use Core's
stdin-JSON/stdout-envelope script convention. The full CLI JSON is retained as
`context.media`; errors retain both the stable native error and process exit code.

Long media execution is marked `execution: async` using Core's existing metadata.
Core task IDs and CLI job-state paths have distinct owners: poll the Core task for
the gateway invocation, and use media `status` / `cancel` for the enhancement.
A cancellation acknowledgement never proves terminal cancellation. Read the final
CLI state and manifest. No detached queue or automatic retry of pixel work exists.

Default adapter outer budget is 900 seconds, configurable up to 86400. CLI child
budgets still apply independently. On adapter outer timeout, or cancellation delivered to an in-process Core
executor, it terminates the CLI supervisor; Unix gets its installed termination handler and Windows gets
Job Object cleanup. Abrupt host kills on macOS inherit the limitation documented
in PLATFORMS.md. Never equate a transport timeout with completed media processing.

## Related existing contracts

- Core SDK: https://github.com/dcc-mcp/dcc-mcp-core/tree/v0.20.44/python/dcc_mcp_core
- Core media skill examples: https://github.com/dcc-mcp/dcc-mcp-core/tree/v0.20.44/python/dcc_mcp_core/skills/media
- Core async skill: https://github.com/dcc-mcp/dcc-mcp-core/tree/v0.20.44/examples/skills/async-render-example

Public interface publication, catalog submission, and installation into an actual
user DCC/gateway are separate actions; this change does none of them.


Core 0.20.44's default subprocess script runner does not propagate its cancellation
token to child Python. Cancelling that Core async task is **not proof** that media
processing stopped. Use this skill's explicit `cancel` tool with the CLI state
path, then inspect `status`. The bridge supports Core's in-process `main(**params)`
execution as well as the default subprocess stdin-JSON path; in-process token
cancellation is handled cooperatively. No new Core cancellation protocol is added.


## Reproduce the real gateway acceptance

With the pinned Core SDK and CLI 0.20.44 already installed, and FFmpeg available:

```sh
python tests/test_gateway_e2e.py --binary /absolute/path/to/dcc-media-enhance \
  --cli /absolute/path/to/dcc-mcp-cli --report /chosen/path/acceptance.json
```

This Linux-only wire test owns short-lived gateway and Python host processes,
loopback-only listeners, a private FileRegistry and temporary HOME/XDG paths. It
uses CLI REST calls throughout; each media call must report
`control_route=gateway` and `gateway_stats_recorded=true`. It verifies real 8x8
resize to 16x16, source hashes, Core `--wait`, native status, cancellation of a
bounded real CPU denoise, no cancelled output, and listener/process teardown.
The existing four-platform SDK tests also verify the exact parsed property-key
sets and actual registered Core `ToolAnnotations` values.

For a `create_skill_server` host to join a gateway, its `gateway_port` must name
the existing gateway port. Setting it to zero disables FileRegistry registration,
not merely gateway election. The acceptance test starts its gateway first and
then joins that exact port; it does not edit the machine-wide registry.

Core's default subprocess runner can mark the invocation `completed` while its
returned skill envelope has `success=false, error=CANCELLED`. The CLI then exits
1 on polling that failed media result; `context.exit_code=5` retains the native
cancellation code. Inspect both layers. A terminal Core task alone does not prove
a successful output, nor does a cancellation acknowledgement prove native exit.

The dated compact acceptance receipt is retained under `evidence/`; CI uploads
its own fresh receipt separately from distribution archives. No gateway access
credentials or persistent profile are created by the test.
