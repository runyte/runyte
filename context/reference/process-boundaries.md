<!-- SPDX-License-Identifier: MPL-2.0 -->
# Process boundaries and extension tiers

This is the register of record for choosing a process boundary in Runyte.
The terminal and desktop editions share the editor and its public extension
contracts. Edition is a packaging choice, not an authority grant.

## Internal helpers

Helpers are private engines started by Runyte, not entry points for third-party
integrations. Their wire is tied to the exact Runyte build: there is no version
range or compatibility promise. The owning frontend supplies bounded input and
receives bounded results; a helper has no editor command, buffer mutation or
context-grant authority. Process isolation and resource limits contain crashes,
hangs and resource exhaustion; they are not an operating-system sandbox.

The required launch convention is an argument vector, never a shell, from the
running edition executable: `runyte --helper <role> [role arguments…]`. Engines
that parse untrusted input or can exhaust time or memory run in a killable child
with resource limits and its own process group. Link a new helper into the
edition executable that needs it. An engine may run in-process only when it
handles no untrusted input and cannot block or exhaust the editor. A separate
executable is permitted only for a different code-signing identity, sandbox
profile or architecture; record that reason here before adding it.

The desktop edition owns two helper roles:

| Role | Engine and direction | Transport and bounds | Source |
| --- | --- | --- | --- |
| `pdf` | Runyte starts Hayro for a page or visible-region refinement | Path and render arguments in; bounded JSON header and RGBA bytes out over stdout. Regular input files up to 128 MiB, at most 10,000 pages, 8,388,608 raster pixels, 8 MiB header. Each invocation has a 15-second wall deadline and CPU limit; address space is 1 GiB on Linux and initial mappings plus 1 GiB on macOS. | `crates/runyte-native/src/pdf.rs`, `crates/runyte-native/src/media.rs` |
| `preview` | Runyte starts Blitz for captured document text | JSON requests on stdin; bounded metadata and RGBA replies on stdout; stderr capture is bounded. Captures and assets, raster dimensions and output bytes have separate limits. Requests have a five-second wall deadline and Linux has a 2 GiB address-space limit. | `crates/runyte-native/src/preview.rs`, `src/document_preview.rs`, `crates/runyte-preview/src/` |

The detailed rendering and asset limits live in
[native window](desktop-edition.md) and
[document preview](native-document-preview.md). Poppler is an external fallback
program, not a bundled internal helper.

Both roles run from the desktop executable through
`crates/runyte-native/src/helper.rs`. Linux uses `/proc/self/exe` to retain the
running build after an on-disk update. Other systems (and Linux without procfs)
compare the captured executable identity before spawning and request a restart
if it changed or became unreadable. All helper streams are piped, stderr is
retained up to 4 KiB while excess bytes are drained, and process groups are
killed before their leaders are reaped. PDF restart errors bypass Poppler fallback.

## Plugins: `runyte-1`

This is a public extension surface. Runyte starts explicitly configured plugin
programs with argument vectors, using newline-delimited JSON over stdin/stdout.
Third-party programs negotiate the stable protocol, supported Runyte release
ranges, features and configured capability grants. The host enforces authority,
captured targets, revisions and resource ownership; a plugin does not inherit
private frontend or context permissions.

Messages are limited to 1 MiB including the newline. Request, queue, resource
and lifecycle bounds are part of the public contract. Use
[the plugin documentation](../../docs/plugins.md) and
[compatibility register](plugin-compatibility.md) for the complete versioning,
authority and bounds rules rather than defining another contract here.
Wire values and workers live in `src/plugin.rs` and `src/plugin/`; host admission
and result application live in `src/workspace/host/plugins.rs` and the adjacent
`plugin_*.rs` modules, with native workflows in `src/app/plugin_*.rs`.

## Context profile: `runyte.context.v1`, and `runyte mcp`

This is a public extension surface for external agents connecting into Runyte.
It uses a separately authenticated, owner-private Unix socket or Windows named
pipe, enabled by a native grant for an exact workspace. It negotiates the
`runyte-1` envelope, a supported release range and the `runyte.context.v1`
feature; ordinary plugin grants cannot authorize it.

The host checks peer ownership, credentials and granted scopes for reads,
revision-checked buffer edits and individually approved terminal text proposals.
Terminal insertion never submits Enter. Grants and approvals remain native
editor actions. Frames are at most 2 MiB; readers, outstanding requests,
snapshots, retained bytes and proposals have independent quotas.

[`runyte mcp`](../../docs/mcp.md) adapts MCP over stdio to this same context
transport. An external client starts the adapter; the adapter connects to
granted hosts and gains no additional authority. Its nine-tool catalog is
distinct from the fuller context operation catalog. The
[context contract](../../docs/plugins/context.md) defines the profile and its
bounds, and the MCP guide defines adapter discovery and usage.
Implementation: `src/workspace/context/`, `src/workspace/host/context.rs`,
`context_reads.rs` beside it, `src/app/context_access.rs`, and `src/mcp.rs` /
`src/mcp/`.

## Private client/host protocol

Bundled Runyte clients connect to persistent-session hosts over owner-private
local Unix sockets or Windows named pipes. A client may start its host. The
same private wire serves terminal and window attachments, lifecycle operations
and `--wait`. It is not an extension surface and has no third-party compatibility
promise.

Admission matches the Runyte library version and the private protocol's
handshake requirements. Terminal and desktop builds of the same version must
interoperate. Host-side behavior must not depend on the host's edition for
features rendered by the current client: a desktop window can attach to a host
built as the terminal edition. Attachment capabilities describe that client.

The host owns editor state and validates connection roles, attachment ownership,
revisions and native input authority. This privileged bundled-client wire is
not interchangeable with scoped context access. Frames, queues, path counts,
text, transactions and connection lifetimes are bounded by the private DTOs and
transport. Source: `src/protocol/`, `src/workspace/transport.rs` and its transport
support modules, and `src/workspace/host/`.

## Choosing a tier

- Third-party code belongs in a plugin or a context client, according to who
  starts the connection and which authority it needs.
- An engine isolated inside Runyte belongs in an internal helper.
- Raster frames and GPU resources never travel over `runyte-1`, whose JSON
  messages have a 1 MiB limit.
- A new public wire requires its own plan. The private client/host wire is not
  a shortcut to extending a public contract.
