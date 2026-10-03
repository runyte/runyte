# Native MCP usability review

Examined `dev` at `1778900` on 2026-10-03. Implementation is uncommitted and
awaits interactive testing. No agent configuration or installed binary was
changed.

## Diagnosis

The Python bridge exposes up to 19 tools, including six snapshot lifecycle
tools alongside ordinary reads. A caller first discovers a workspace, lists
resources, then copies both workspace and long connection-bound resource
handles into later calls. Finding a terminal by its contents requires separate
reads when the title does not identify the running agent. These are additional
model decisions and tool round trips; they are not evidence that a healthy
host spends two minutes processing a request.

Write tools are advertised only after scopes are discovered. Clients that
cache the startup tool list cannot use a later write grant, even though native
authorization succeeds. The deferred tool-list-refresh issue documents this
client behavior. Probing scopes before the first tool list helps only when
permissions already exist and adds discovery work to startup.

The permission overlay uses wire scope names, initially selects `agent` even
when another identity owns the active grant, and initializes Remember to false
when reopening a remembered grant. It lets dependent write and read toggles
become inconsistent, with refusal deferred until submission.

## Implemented design

`runyte mcp --identity <name>` is an early stdio mode of the Rust binary. It
starts neither an editor nor its configuration/logging/terminal lifecycle.
It uses the existing private discovery, credential storage and verified local
transport directly. It has no new dependencies or Python runtime requirement.
The Python adapter remains available for compatibility and advanced snapshot,
selection and viewport tools.

The native adapter advertises nine stable tools. Tool presence describes the
API; authorization still belongs to the exact workspace host on every call.
This deliberately replaces grant-dependent advertisement for the native
adapter and removes the need for client tool-list refresh. `list_workspaces`
checks permissions; `find_resources` also returns usable resource handles.
Healthy connections and handles survive repeated searches. Grants changed
natively close existing readers; explicit discovery renews those connections.
Mutations never reconnect or replay automatically.

`find_resources` combines workspace discovery, inventory and fuzzy matching of
names and contents. It uses the editor's smart-case line matcher and returns
matching excerpts and allowed actions. A terminal named `shell` can be found
from its Claude/Codex output. Several candidates remain explicit candidates;
no write is routed by a fuzzy name or the current focused pane. Subsequent
tools take one short opaque resource handle without repeating a workspace.
The typical route is search then read/append/propose. Proposal responses tell
the agent to let the person review instead of polling for human input.

The search reads bounded text through existing authorized host operations,
not a new scan in the editor loop. Defaults are four workspaces per discovery
page, eight buffers and eight terminals per workspace, 8,192 buffer characters,
and 80 recent terminal rows. There is a three-second budget after discovery
and authentication. Pagination and the buffer content offset are explicit;
results report partial search and errors. These bounds can miss older terminal
text or later buffer text; no result is not proof of absence. Source text is
not retained in the adapter's handle metadata.

`:mcp [identity]` is the canonical registry command; `:context-access` remains
an alias. The overlay shows current grant lifetime and connected reader count,
restores Remember, offers identity cycling, and maintains read prerequisites
when toggling write permissions. Physical approval and the separate Enter
required to submit terminal text are preserved.

## Verification

The original 30 Python bridge tests passed before implementation. Rust tests
exercise content matches under unrelated titles, Unicode character ranges,
later buffer windows, stable catalog and handles, absent storage, late grants,
revocation, foreign handles, malformed/lost mutation replies, terminal control
rejection, strict JSON framing, and CLI startup before any editor initialization.
Native permission tests cover dependency toggles, identity cycling and restored
remembered state, alongside the existing physical-input approval tests.

`bridges/runyte-context/tests/test_native_mcp.py` runs the built-in stdio mode
against real editors with isolated storage. It covers fuzzy terminal contents,
unsaved appends, pending/cancelled proposals, detached persistent workspaces,
independent clients, grants made after startup, and revocation/regrant without
restarting the MCP process. The older Python acceptance fixtures remain in the
suite to verify the shared host transport and command alias.

Commands:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo llvm-cov --locked --workspace
cargo build --locked
cd bridges/runyte-context
RUNYTE_CONTEXT_TEST_BINARY="$PWD/../../target/debug/runyte" python3 -m unittest discover -s tests -v
```

The Python acceptance suite needs the pinned `benchmarks/requirements.txt`
screen-decoder dependencies. All sockets, permissions, configurations and
editor children belong to temporary fixtures.

Initial local Linux measurements with the debug binary: native process startup
plus initialization and tool listing took 3.8 ms. Five fuzzy content searches
across two live terminal panes took 3.4–5.9 ms. These are host/transport timings,
not measurements of model reasoning or MCP-client approval latency. A weak-model
interactive trial remains necessary to judge the complete user experience.

The first canonical Linux coverage run measured 91.76% total lines, above the
unchanged 89% floor. The final canonical run passed all 4,417 tests (41
ignored) and measured 144,763 of 157,660 lines covered: **91.82%**. The
ordinary `cargo test` run also passed 4,417 tests with 41 ignored. Formatting,
all-target Clippy with warnings denied, and the debug build passed.

The final real-editor acceptance run passed 50 tests with 7 Windows-specific
skips. With the final debug binary, native startup plus tool listing took
4.3 ms, versus 29.8 ms for the Python adapter in the same fixture. Native
content searches took 3.2–5.5 ms across five samples. Serialized tool
descriptions occupied 7,199 bytes for nine native tools versus 12,858 bytes
for 19 legacy tools (about 44% less). These are single-fixture observations,
not general performance guarantees.

Native macOS and Windows execution was not available. A Windows GNU cross-check
could not build grammar dependencies because `x86_64-w64-mingw32-gcc` is absent;
it provides no Windows validation evidence.

The stable catalog follows MCP's separation of tool definitions and invocation
errors; it does not depend on optional list-change notifications. See the
[MCP tools specification](https://modelcontextprotocol.io/specification/2025-06-18/server/tools).
