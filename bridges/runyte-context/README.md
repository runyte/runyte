# Runyte context MCP bridge

This separately versioned Python package connects a local MCP client to
explicitly authorized Runyte workspaces. It reads live terminal screens,
retained terminal history, unsaved buffers, selections, and native viewports.
It also exposes revision-checked buffer edits, atomic appends and proposals for terminal text
when the corresponding native grants exist. Runyte owns every authorization
decision and terminal approval.

Requires Python 3.11 or later and a Runyte host supporting
`runyte.context.v1`. Linux, macOS, and x86-64 Windows 11 are supported. There
are no runtime dependencies or account connections.
The Rust editor has no dependency on this package or an MCP runtime. Its
package version and release lifecycle are independent of the editor.

## Install and pair

With [uv](https://docs.astral.sh/uv/), install the bridge as a tool from this
directory:

```sh
uv tool install .
uv tool dir --bin
```

The second command prints the directory holding the `runyte-context`
executable, usually `.local/bin` in the account home. uv selects or downloads
a suitable Python itself. The install is a copy: after changing the bridge,
run `uv tool install --reinstall .` again. While developing the bridge,
`uv tool install --editable .` runs the source in place instead.

Without uv, install into a virtual environment using only Python:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install .
```

The executable is then `.venv/bin/runyte-context` in this directory.
Alternatively, run `python3 -m runyte_context` directly from this directory
without installing anything. Use an absolute executable path in MCP client
configuration so it works from every workspace.

On Windows, create the virtual environment and install the bridge from
PowerShell with:

```powershell
py -3.11 -m venv .venv
.\.venv\Scripts\python.exe -m pip install .
```

The executable is `.venv\Scripts\runyte-context.exe`. You can also run
`.\.venv\Scripts\python.exe -m runyte_context` from this directory. Use the
absolute path to the executable or Python interpreter in MCP client
configuration.

In each Runyte workspace you want to expose, open `:context-access codex` or
`:context-access claude`, review the requested scopes, and approve in Runyte.
The confirmation requires physical frontend input. An agent or bridge request
cannot approve itself. Reopen `:context-access` and press `x` to revoke an
identity immediately; revocation disconnects its readers and removes any
remembered grant.
Use a separate identity name for each agent when their grants should differ.
The bridge only reads the resulting private credential file; it never creates
credentials or grants. Credentials are not command-line arguments and must not
be copied into client configuration.

The default private store is the account home’s `.cache/runyte/context` on
Linux, `Library/Caches/runyte/context` on macOS, and `runyte\context` below the
account's LocalAppData known folder on Windows. Windows resolves that folder
through the operating system rather than trusting `%LOCALAPPDATA%`. An absolute
`RUNYTE_CONTEXT_HOME` overrides the default for both Runyte and the bridge. On
Windows its immediate parent must already exist and the store must be on local
NTFS; Runyte protects it with an owner-only ACL and refuses reparse or hardlink
traversal. When using isolated environments, configure the same value in each
participating process.
Ordinary discovery respects Runyte’s environment boundary;
`list_workspaces(include_hidden=true)` is an explicit opt-in to broader
inventory discovery, with independent authentication at each host.

## Configure clients

For Codex, add the following table to `~/.codex/config.toml`, replacing the
example executable path with the installed bridge’s absolute path from either
installation method. Codex reads `$CODEX_HOME/config.toml` instead when
`CODEX_HOME` is set. The file applies to every Codex session for the account;
create it if it does not exist, and restart Codex after editing it:

```toml
[mcp_servers.runyte]
command = "/path/to/bin/runyte-context"
args = ["--identity", "codex"]
```

This uses Codex’s documented
[stdio MCP configuration](https://developers.openai.com/codex/mcp).

For Claude Code:

```sh
claude mcp add --scope user --transport stdio runyte -- /path/to/bin/runyte-context --identity claude
```

The command writes the entry to `~/.claude.json` rather than to a file in the
project; edit it through `claude mcp` instead of by hand. `--scope user` makes
the server available to Claude Code in every directory, like the Codex entry
above. Omit it to register the bridge only for the project where the command is
run, which is Claude Code’s default. Confirm the registration with
`claude mcp list`, and start a new Claude Code session to load it. The scope,
command and argument separator follow Claude Code’s
[MCP configuration documentation](https://code.claude.com/docs/en/mcp).

Other MCP stdio clients can use this server entry:

```json
{"mcpServers":{"runyte":{"command":"/path/to/bin/runyte-context","args":["--identity","agent"]}}}
```

`--runyte /absolute/path/to/runyte` selects the discovery executable;
`--timeout 2` sets the per-host deadline in seconds, between 0.1 and 10.
Each client launches its own bridge process. Installation and configuration
are local; the examples do not publish a package or contact an agent account.

On Windows, pass the absolute path to `runyte.exe` when it is not on `PATH`,
for example `--runyte C:\Tools\Runyte\runyte.exe`. Discovery invokes
`runyte.exe --context-list --json`; the bridge then authenticates each selected
workspace over its private local named pipe. Pipe addresses are discovered,
not copied into MCP configuration, and no TCP listener is opened.

## Use

The initialization response tells the agent to start with `list_workspaces` and
normally choose the returned workspace whose root matches its current project.
Name another workspace when that is the intended target. The four common routes
are:

- buffer read: `list_buffers`, then `read_buffer`;
- terminal read: `list_terminals`, then `read_terminal`;
- buffer write: `list_buffers`, then `edit_buffer` with the listed revision,
  or `append_buffer` to add at the end without one;
- terminal write: `list_terminals`, then `propose_terminal_text`, followed by
  `terminal_proposal_status`.

For example:

> Read Claude’s terminal in workspace B and summarize its latest findings.

Names can be duplicated. Resource handles bind the exact host incarnation and
connection, and every subsequent call requires the workspace handle as well.
A reconnect, host restart, or connection eviction requires fresh resource
discovery. Matching the current project chooses among discovered workspaces; it
does not bypass discovery or fall back to the currently focused workspace.

| Tools | Native scope |
| --- | --- |
| `list_terminals`, `read_terminal`, terminal snapshot open/read/close | `terminal_read` |
| `list_buffers`, `list_panes`, `read_buffer`, `read_selection`, `read_pane`, buffer snapshot open/read/close | `editor_context_read` |
| `edit_buffer`, `append_buffer` | `buffer_edit` plus editor reads |
| `propose_terminal_text`, `terminal_proposal_status`, `cancel_terminal_proposal` | `terminal_propose` plus terminal reads |

Terminal viewports additionally require terminal reads. Detached persistent
hosts still expose live terminal/buffer content, but have no native viewport.
Read results include structured provenance and mark source content untrusted.
They preserve the host’s revisions and clipping metadata. Terminal rows are
presentation text, not a reconstructed conversation or shell transcript.

Mutation tools appear only when a discovered workspace has the corresponding
grant. Before answering a client’s first tool-list request, the bridge runs the
same bounded discovery as `list_workspaces` once and keeps only the granted
scopes, so grants that already exist are offered from the start. It does not
make any workspace usable: calls still need a handle returned by
`list_workspaces`. If discovery fails or a host does not answer in time, the
first list has the read tools and whatever the responsive hosts granted.
Later grant changes send `notifications/tools/list_changed`. Each invocation
still checks the selected target’s grant; access to one workspace never grants
access to another.

Some clients do not act on that notification. Codex 0.155.0 logs it but keeps
the tool list it fetched at startup, so it sees the write tools only if the
grant existed when the session started; after granting access during a Codex
session, start a new one. A revocation during the session leaves the tools
listed, and calling them fails. Claude Code refreshes its tool list when
notified.

`edit_buffer` allows multiline text in one atomic transaction and never saves
or completes an external-editor wait. `append_buffer` adds text at the buffer's
current end without a revision. Concurrent appends from several agents are
applied one at a time and all are kept. An optional `expected_tail` must match
the buffer's current ending, or nothing is written and the call fails as
`stale`. The result gives the inserted scalar range, the number of line breaks
and a preview of up to 256 scalars of what was actually inserted, so a literal
`\n` sent in place of a line break is visible. `propose_terminal_text` allows only one
line, with an optional short `reason` displayed separately as untrusted text.
It creates a pending native overlay; no text reaches the terminal before
the person chooses **Insert text**. That approval inserts text without Enter.
The person submits separately. No tool accepts a submit flag, raw keys,
generic host methods, or approval commands. Printable characters can still
trigger actions in arbitrary terminal programs.

Use snapshot open/read/close tools for consistent paging while output changes.
Default terminal reads request 200 rows, 64 KiB of text and 64 Ki cells; the
host’s maximum is 1,000 rows and 256 KiB/cells. The bridge retains at most eight
live workspace connections with least-recently-used eviction and never caches
source text. Workspace listing is paginated and authenticates candidates with
bounded parallel probes; unavailable hosts are reported individually. Each
page admits at most the currently available connection slots (up to eight)
as fresh probes, with one deadline spanning the complete handshake. Follow
`next` to authenticate subsequent candidates; a page may be shorter than
its requested limit. If all
eight connection slots are occupied, undiscovered targets are listed with
`connection_limit`; selecting one through an inventory tool evicts the oldest
connection and authenticates that target explicitly.

A transport failure during a mutation reports `outcome_unknown`. Do not retry
automatically: the host may already have received the request. Read failures
also do not silently reconnect or reuse obsolete resource handles. Revocation
is enforced by the host, including responses already queued but not delivered.

## Protocol and tests

The bridge implements MCP `2025-06-18` over
[newline-delimited stdio](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports),
with [initialization](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
and [tool calls](https://modelcontextprotocol.io/specification/2025-06-18/server/tools).
Other versions receive the supported version during negotiation. Stdout carries
only protocol messages. JSON input/output and subprocess discovery are bounded
to 2 MiB. A tool result includes both structured content and its text equivalent;
if that combined representation exceeds the frame ceiling, request fewer rows
or bytes. Requests are processed serially; deadlines bound a blocked host call.

Run the independent, network-free suite from this directory:

```sh
python3 -m pip install -r ../../benchmarks/requirements.txt # Unix screen-decoder tests only
python3 -m unittest discover -s tests -v
```

The Unix fixture reuses the benchmark terminal decoder to observe completed
screen frames, including cursor-addressed redraws. These are test dependencies;
the installed context bridge still has no runtime dependencies. Windows's
native fixtures do not use this decoder.

Fixtures run real MCP stdio clients against independent local hosts, including
a detached host, without invoking agent accounts. They cover explicit
routing, connection ownership, grant revocation, immutable paging, multiline
buffer edits, pending proposals, control rejection, lost acknowledgments,
bounded discovery, private storage and connection eviction. Native physical
approval and real PTY delivery are covered by Runyte’s Rust tests. Windows
acceptance uses native named pipes; Unix acceptance uses local sockets.

To include the real editor integration test, first build Runyte and supply its
absolute binary path (from this directory):

```sh
cargo build --manifest-path ../../Cargo.toml
RUNYTE_CONTEXT_TEST_BINARY="$PWD/../../target/debug/runyte" python3 -m unittest discover -s tests -v
```

This test starts two real editor hosts and two MCP clients with temporary,
private identity and grant fixtures. Both agents read separate live terminal
panes and a detached persistent workspace. A multiline buffer edit remains
unsaved and visible to the other agent; native revocation denies only the
selected identity and workspace. Every process and storage path belongs to the
fixture. Linux and macOS CI run this test after building the editor.
