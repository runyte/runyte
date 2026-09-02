# Workspace search blocks input and rendering while it scans files

`Space / s` and `Space / S` open their prompts without blocking, but accepting
either prompt can make the editor unresponsive until the workspace search has
finished. During that interval Runyte cannot process input, redraw, or drain
other service events. The delay grows with the number and size of files below
the workspace root and is especially visible on slow or remote filesystems.

To reproduce, open a workspace containing enough files to make a recursive
content scan noticeable, press `Space / s` or `Space / S`, enter a pattern, and
press Enter. Try to move, cancel, or resize Runyte before the result buffer
appears. No input or frame is handled until the scan completes.

The prompt-acceptance branch in `src/app/input.rs` calls
`App::open_global_search` directly. That function, in
`src/app/language_workflows.rs`, compiles the query and immediately calls
`workspace_matches`. `workspace_matches`, in `src/app.rs`, performs the whole
recursive walk synchronously: it calls `std::fs::read_dir`, reads metadata,
loads each admitted file with `std::fs::read_to_string`, and matches every line
before returning. `open_global_search` then reconciles disk matches with live
open buffers, sorts and truncates the results, builds the generated document,
and retargets the active pane. This entire call chain runs inside key handling
on the host thread.

The 4 MiB per-file limit and 10,000-result limit bound individual inputs and
the returned result set, but they do not bound the number of directories and
files visited or the total bytes read before a result is produced. They
therefore do not protect event-loop responsiveness.

## Potential solution

Move workspace searching behind an asynchronous service boundary, following
the file scanner's request/event model without changing workspace search into
a picker. Accepting the prompt should validate the query, allocate a monotonic
request identity, record a visible pending status, and return to the event
loop. A worker should walk the filesystem and send a bounded completed or
failed event. Starting a newer workspace search should supersede the older
request, and the worker should check cancellation between directories, files,
and batches of matches so abandoned scans do not continue consuming I/O.
Completion must be tagged with the request identity and workspace root so a
late result cannot replace a newer search or a different workspace's view.

Open buffers must remain authoritative over their on-disk paths. A request can
exclude those paths from the disk walk and carry cheap cloned `Text`/rope
snapshots for the worker to search, rather than calling `Buffer::to_string` on
the host thread. This also gives the retained result buffer one coherent
query-time snapshot while allowing later edits to proceed. If the rope values
cannot safely cross the chosen service boundary, live-buffer matching should
instead advance in bounded event-loop slices, as finder live-content scanning
does; copying and scanning complete open buffers synchronously would leave a
second form of the same stall.

The worker may return already ordered, bounded matches, or a fully assembled
result document plus its typed `WorkspaceSearchTarget` rows. The host should do
only bounded result application: verify the request, replace or create the
singleton `[workspace search]` special buffer, preserve any matching selected
target when rebuilding it, and retarget the active pane. Filesystem failures
must arrive as service failures rather than being hidden by cancellation.

The implementation must preserve the current search contract:

- `Space / s` is a case-insensitive escaped literal and `Space / S` is a
  line-scoped regular expression;
- `.git`, `.runyte`, `target`, symlinks, configured hidden-file behavior, the
  4 MiB file limit, and the 10,000-result limit retain their current meanings;
- workspace search continues to ignore `.gitignore` and `.ignore` files;
- open buffer text wins over disk text for the same path;
- results remain a retained, read-only special buffer with typed jump targets,
  not a live picker overlay; and
- standalone and persistent hosts drain the same service events and remain
  responsive while a request is pending.

Regression coverage should use a controllable scanner seam or worker fixture
rather than timing a real filesystem. It should prove that prompt acceptance
returns before scan completion, input and snapshots continue while the request
is pending, a newer request rejects a late older result, cancellation does not
publish a failure, a real failure is reported, and unsaved open-buffer content
still replaces disk matches. Existing workspace-search result-buffer, limit,
escaping, flavour, and jump tests in `src/app/tests/editing_and_buffers.rs`
should continue to pass through the asynchronous completion path.
