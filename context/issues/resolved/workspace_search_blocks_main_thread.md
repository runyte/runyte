---
title: "Workspace search blocked input and rendering during file traversal"
status: resolved
reported: 2026-09-02
resolved: 2026-09-02
commit: 8b259be
---

## Resolution

Commit `8b259be` (`Search workspaces off the host thread`) moved retained
workspace search behind a cancellable background service. The incorrect
boundary was `App::open_global_search` in `src/app/language_workflows.rs`: the
prompt-acceptance path called `workspace_matches` directly, so recursive
directory reads, file reads, matching, open-buffer reconciliation, sorting,
and result construction all ran inside key handling on the host thread.

`src/workspace_search.rs` now owns filesystem traversal and text matching. A
request carries a monotonic identity, the compiled matcher, the workspace
scope, and cheap cloned `Text` snapshots for open files. Rope cloning shares
the underlying chunks, so unsaved text remains authoritative without copying
or scanning complete buffers on the host thread. The worker preserves the
existing exclusions, file and result bounds, line-scoped matching, result
order, and disk-scan truncation behavior. It publishes one bounded completion
or failure event and checks an atomic request identity between directories,
files, rows, and match batches. Starting a newer request supersedes the older
one.

Both standalone and persistent event loops drain `WorkspaceSearchEvent`
through the shared `HostEvent` boundary. The application accepts a completion
only when its identity is still pending, then performs the bounded work of
creating or replacing the singleton `[workspace search]` special buffer and
retargeting the pane. Prompt acceptance therefore returns immediately with a
`searching workspace in the background` status, while late results cannot
replace a newer query. A synchronous service-free seam remains for isolated
application tests and embedders that deliberately construct `App` without its
host services.

Regression coverage is in:

- `src/workspace_search.rs`:
  `rope_matching_has_string_lines_semantics`,
  `cancellation_produces_no_result_or_failure`,
  `traversal_failures_remain_failures`, and
  `threaded_service_returns_a_bounded_identified_event` cover rope snapshot
  matching, cooperative cancellation, error propagation, request identity,
  and the real worker channel;
- `src/app/tests/editing_and_buffers.rs`:
  `workspace_search_returns_to_input_before_controlled_scan_completion`
  proves that input remains live while a deterministic request is held,
  `workspace_search_rejects_a_superseded_completion` covers stale completion
  rejection, and `background_workspace_search_uses_the_open_buffer_snapshot`
  covers unsaved-buffer authority through asynchronous result application;
  and
- the existing workspace-search result-buffer, path escaping, result-limit,
  flavour, rebuild, and jump tests in
  `src/app/tests/editing_and_buffers.rs` retain the previous behavior.

The change passes `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

Known limitation: cancellation is cooperative. It cannot interrupt one
in-progress operating-system directory enumeration or file read, but it is
observed before the next entry or row; admitted files remain bounded to 4 MiB.

## Report

`Space / s` and `Space / S` opened their prompts without blocking, but
accepting either prompt could make the editor unresponsive until the workspace
search finished. During that interval Runyte could not process input, redraw,
or drain other service events. The delay grew with the number and size of
files below the workspace root and was especially visible on slow or remote
filesystems.

To reproduce, open a workspace containing enough files to make a recursive
content scan noticeable, press `Space / s` or `Space / S`, enter a pattern,
and press Enter. Attempt to move, cancel, or resize Runyte before the result
buffer appears. No input or frame was handled until the scan completed.

The prompt-acceptance branch in `src/app/input.rs` called
`App::open_global_search` directly. That function, in
`src/app/language_workflows.rs`, compiled the query and immediately called
`workspace_matches`. `workspace_matches`, then in `src/app.rs`, performed the
whole recursive walk synchronously: it called `std::fs::read_dir`, read
metadata, loaded each admitted file with `std::fs::read_to_string`, and matched
every line before returning. `open_global_search` then reconciled disk matches
with live open buffers, sorted and truncated the results, built the generated
document, and retargeted the active pane. This entire call chain ran inside key
handling on the host thread.

The 4 MiB per-file limit and 10,000-result limit bounded individual inputs and
the returned result set, but they did not bound the number of directories and
files visited or the total bytes read before a result was produced. They
therefore did not protect event-loop responsiveness.

The potential solution was an asynchronous service following the file
scanner's request/event model without turning workspace search into a picker.
Prompt acceptance needed to validate the query, allocate a monotonic request
identity, record visible pending status, and return to the event loop. The
worker needed to publish a bounded completed or failed event, support
cooperative cancellation, and tag completion with both request and workspace
identity so a late result could not replace a newer search or a different
workspace's view.

Open buffers needed to remain authoritative over their on-disk paths without
copying and scanning their complete text synchronously. Cheap cloned
`Text`/rope snapshots were one safe option. If those values could not cross
the service boundary, live-buffer matching instead needed to advance in
bounded event-loop slices like finder live-content scanning. The host was to
perform only bounded result application: verify the request, replace or create
the singleton special buffer, preserve a matching selected target when
rebuilding it, and retarget the active pane. Filesystem failures needed to
arrive as service failures rather than being hidden by cancellation.

The following behavior had to remain unchanged:

- `Space / s` is a case-insensitive escaped literal and `Space / S` is a
  line-scoped regular expression;
- `.git`, `.runyte`, `target`, symlinks, configured hidden-file behavior, the
  4 MiB file limit, and the 10,000-result limit retain their meanings;
- workspace search does not consult `.gitignore` or `.ignore` files;
- open buffer text wins over disk text for the same path;
- results remain a retained, read-only special buffer with typed jump targets,
  not a live picker overlay; and
- standalone and persistent hosts drain the same service events and remain
  responsive while a request is pending.

Regression coverage needed a controllable scanner seam or worker fixture
rather than filesystem timing. It needed to show that prompt acceptance
returned before completion, input and snapshots continued while pending, a
newer request rejected a late older result, cancellation published no failure,
a real failure was reported, and unsaved open-buffer content still replaced
disk matches. The existing workspace-search result-buffer, limit, escaping,
flavour, and jump tests needed to continue through the asynchronous completion
path.
