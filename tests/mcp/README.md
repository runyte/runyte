# Built-in MCP acceptance

These tests launch `runyte mcp --identity <name>`. Python is only the test
harness; there is no Python MCP server or runtime package.

On Linux or macOS, build Runyte and install the PTY screen parser in a temporary
virtual environment:

```sh
cargo build --locked
python3 -m venv /tmp/runyte-mcp-tests
/tmp/runyte-mcp-tests/bin/python -m pip install -r benchmarks/requirements.txt
RUNYTE_CONTEXT_TEST_BINARY="$PWD/target/debug/runyte" \
  /tmp/runyte-mcp-tests/bin/python tests/mcp/run.py
```

The runner requires an executable binary and fails on empty or skipped suites.
It runs native discovery, permissions before/after startup, exact targets,
independent clients, detached workspaces, unsaved edits, concurrent atomic
appends, revocation, and the editor/PTY synchronization regressions. All editor
storage and grants are fixture-owned. Local sockets and PTYs must be permitted.
`editor_fixture.py`, `native_pty.py`, `workspace_readiness.py`, and
`mcp_client.py` provide reusable test fixtures. The client speaks only MCP
stdio to the Runyte binary; it does not implement the host context protocol.

On Windows, the Rust ConPTY driver owns physical grants, remembered-grant
restart and revocation, and launches `test_windows_native.py` against the
public executable:

```powershell
$env:RUNYTE_CONTEXT_TEST_PYTHON = (Get-Command python).Source
cargo test --locked --test windows_context_acceptance public_windows_context_round_trip -- --ignored --exact --nocapture
```

CI verifies that this named test ran. The Python scenario fails if its fixture
environment is absent and the Rust driver rejects skipped acceptance results.
It needs only the Python standard library on Windows.

Host operations outside the nine-tool MCP catalog remain covered by
[src/workspace/host/tests/context_reads.rs](../../src/workspace/host/tests/context_reads.rs):
pane listing, selection/viewport ownership and invalidation, immutable buffer
and terminal snapshots, Unicode offsets, and retention budgets. Native
transport, authentication, revocation and transactional-edit coverage remains
in `src/workspace/host/tests/context_transport*.rs` and
`src/workspace/context/tests/`. Those tests do not depend on an MCP adapter.
`real_pty_overlay_enter_only_inserts_and_separate_native_enter_submits` in
`src/workspace/host/tests/context_access.rs` and the native-input tests retain
terminal proposal approval and insertion-without-Enter coverage. Run the canonical `cargo test` suite for these
boundaries; `tests/mcp_cli.rs` and `src/mcp/tests*` cover the Rust adapter itself.
