# Externally changed file buffers are not reported until save

Runyte records enough information to prevent an ordinary save from silently
overwriting a file that changed after the buffer was opened or last saved.
`Buffer::save` compares the current file with the recorded `DiskState`, which
includes content, metadata, and file identity, and refuses a mismatch. The
atomic replacement path repeats the relevant checks around installation.

That guard is reactive. While a file remains open, an external editor,
formatter, Git operation, build tool, or agent can replace it without any
visible change in Runyte. The pane continues to show the old buffer text and
the user normally discovers the disagreement only after `:write` fails.
Furthermore, `:reload` currently replaces a dirty file buffer immediately and
clears its undo history. A mistaken `Space r` can therefore discard unsaved
buffer text without confirmation.

## Expected behavior

Runyte detects when an open file buffer no longer agrees with its path and
reports the condition without modifying the buffer. Every pane showing that
buffer carries a `[STALE]` marker. The active buffer carries the same state in
the global status line, and the first observation of each distinct external
revision creates one retained WARNING notification. A suitable notification
is:

```text
src/app.rs changed on disk · Space b d compares · Space r reloads
```

The markers are independent:

```text
[file] src/app.rs [+] [STALE]
```

`[+]` means the in-memory text differs from its accepted baseline. `[STALE]`
means the path no longer agrees with the disk state Runyte accepted for that
buffer. `[RO]` retains its existing meaning. Pane-title marker order is `[+]`,
`[STALE]`, `[RO]`, followed by a maximized-view marker when one exists.

Detection must never replace buffer text, move a selection, clear undo, mark a
buffer clean, or write to disk. The existing save-time `DiskState` and atomic
replacement checks remain the final authority even after proactive monitoring
is added.

## Reconciliation model

Reconciliation compares three values:

- the baseline accepted when the file was opened, reloaded, or last saved;
- the current in-memory buffer text; and
- one complete observation of the current path, containing its text and
  `DiskState` from the same open file handle.

The resulting states are:

| Condition | Meaning | Behavior |
| --- | --- | --- |
| disk equals baseline | Synchronized | Clear any obsolete external-change state. |
| buffer equals baseline and disk differs | Disk-only change | Mark `[STALE]`; offer reload or comparison. |
| buffer and disk both differ from the baseline and from each other | Conflict | Preserve the buffer; mark `[+] [STALE]`; offer comparison or confirmed reload. |
| buffer text equals observed disk text | Converged | Adopt the observed `DiskState` and current text as saved without clearing undo history, then clear `[STALE]` and `[+]`. |

Deletion, replacement by binary data, and a stably unreadable path are separate
external states. They use `[STALE]` but their notification explains the actual
condition. A deleted path has no reload or comparison source; keeping the
buffer and later saving it retains the existing file-recreation behavior. A
binary replacement is not admitted into the text buffer. A transient
not-found or permission error during an atomic replacement must be retried
after the monitor debounce before it is reported as stable state.

Metadata or identity may change while text remains equal. The observed
`DiskState` must still be adopted only through the converged case or explicit
reload, because a later save must not overwrite a permission, ACL, identity,
or symlink-target change using an obsolete expectation.

## Monitoring architecture

Add a host-owned file-monitor service. It belongs beside the existing
asynchronous syntax, file-picker, Git, and workspace services rather than in a
frontend or renderer. Both the standalone loop and the persistent-session
host drive it. A persistent host continues monitoring open file buffers while
the TUI is detached, so attachment receives the retained state and warning.

The service watches the parent directories of open file buffers, deduplicating
directories shared by several files. Watching parents rather than individual
inodes is required because Runyte and common external tools save through
atomic replacement. Filesystem notifications are wake-up hints, not proof of
a change. After a short debounce, the worker reads every affected open path
and constructs text and `DiskState` from one file handle using the same
complete binary classification as `Buffer::open` and `Buffer::reload`.

Use a native cross-platform watcher suitable for the currently supported
Linux and macOS targets. Add a periodic metadata reconciliation pass as a
fallback for lost watcher events and perform an immediate observation before
an explicit disk comparison. The periodic pass should inspect metadata first
and read content only for candidates whose metadata or identity changed;
same-size, timestamp-preserving rewrites may rely on the native event for the
proactive warning, but the save-time digest comparison must continue to catch
them even if the event was lost.

Every observation request carries:

- the durable buffer index;
- the path spelling being observed; and
- a monotonically increasing disk-baseline generation.

Opening, successful saving, successful reloading, save-as, path retargeting,
and closing advance or invalidate that generation as appropriate. A returned
observation is applied only if the buffer is still live, is still a file
buffer for the requested path identity, and still has the request's
generation. This prevents a late worker result from making a newly saved or
reloaded buffer stale. Watcher events caused by Runyte's own atomic save are
therefore harmless: their observation either matches the new baseline or is
rejected as an old generation.

Coalesce queued work per buffer and bound event queues. Repeated observations
of the same external `DiskState` do not create repeated notifications or
snapshot buffers. Closing the last live owner unregisters its path, while
several panes showing the same buffer never create duplicate watches.

The complete observed text may be retained behind an `Arc<str>` while the
state is stale so a notification, confirmation, and comparison refer to one
revision without duplicating large strings. Compact external state belongs to
the file buffer or application buffer-state arena; full observation and
comparison lifecycle belongs to the application rather than the renderer.

## Presentation and protocol

Add a presentation-neutral external-file status enum rather than asking a
frontend to infer staleness from display text. `PaneTitle` and `StatusSnapshot`
carry the state needed to render `[STALE]`. The open-buffer manager also marks
stale buffers so a changed hidden buffer remains discoverable. Update the
private protocol DTOs and conversions together with the core snapshots, and
bump or extend the bundled protocol version according to its existing
compatibility rules.

The `[STALE]` marker persists after the interaction-line message is replaced
and after the warning notification is acknowledged. Only agreement with the
baseline, convergence, successful reload, or a successfully verified save can
clear it. Merely dismissing an overlay or opening and closing a comparison is
not reconciliation.

Update `context/reference/ui-vocabulary.md` so pane titles and the global
status line own this marker. Update `context/reference/helix-keymap-v1.md`
because the implementation adds a buffer command and binding. Document the
behavior, marker, destructive-reload confirmation, comparison, binary and
deleted-file cases, and force-save boundary in `docs/user-guide.md`.

## Reload safety

Change `:reload` for every dirty ordinary file buffer, not only a buffer known
to be stale. A dirty reload opens a confirmation overlay and changes nothing
until Enter. Escape and `Ctrl-c` cancel. The overlay names the path and says
that reloading discards unsaved Runyte changes and clears their undo history.
When `[STALE]` is present it additionally points to `Space b d` as the
non-destructive comparison path.

The confirmation owns the exact disk observation it proposes to install. On
Enter, re-inspect the path and require it to match that observation before
changing the buffer. If the file changed again while the confirmation was
open, retain the buffer, replace the stale observation, and report that reload
must be reviewed again. This follows the confirmation-overlay contract: Enter
must not accept disk contents different from the prepared operation.

A clean file buffer may reload immediately because no in-memory text is lost.
Reading and validation still complete before any buffer, syntax, LSP, pane, or
undo state changes. The existing `resync_replaced_buffer` and shared-pane
normalization paths remain responsible for successful reload aftermath.

An ordinary save of a known stale buffer must refuse before
`editor.trim_trailing_whitespace` or another save hook mutates the buffer. The
atomic save path must recheck the disk independently to cover a change that
occurs after this preflight. If that final guard discovers a previously
unknown mismatch, schedule an immediate observation so `[STALE]` and the
comparison command become available instead of leaving only a generic save
error.

`:write!` remains the explicit command that replaces an externally changed
file with the buffer. It does not need a second confirmation, but it clears
`[STALE]` only after the installed contents and new `DiskState` are verified.

## Disk comparison

Add `:diff-disk` and bind it to `Space b d` in the existing Buffers namespace.
The command is available only for an ordinary file buffer. It requests a fresh
complete disk observation, then compares that immutable observation with the
authoritative in-memory buffer. A changed file is not required; invoking the
command on a synchronized file may report that the two sides are identical.

Represent the observed disk contents as a read-only generated buffer with a
stable identity tied to the source buffer and observed disk revision. Its pane
title is `[disk] <path> [RO]`. Reuse the existing side-by-side `DiffSession`
and alignment implementation instead of generating a separate patch or
building a second comparison renderer. Put the disk snapshot on the left and
the editable Runyte buffer on the right, so right-side additions and changes
describe the text the user is protecting. The source remains the same buffer:
it keeps selections, undo history, dirty state, LSP ownership, and normal edit
behavior, and the live comparison follows further edits on that side.

If the source is already visible, create or reuse another pane for the disk
snapshot following the existing `:diff-this` layout rules. Refuse cleanly when
there is no room, when either side exceeds `MAX_DIFF_BYTES`, when the disk
version is binary, or when the file is deleted or unreadable. Failure must not
retarget a pane or modify the source.

The disk snapshot does not update silently while it is being read. If another
external revision arrives, retain the snapshot as the revision currently under
comparison and mark the source stale for the newer revision. Running
`:diff-disk` again replaces or opens the comparison for the latest complete
observation. If `:write!` overwrites the external revision while its snapshot
is visible, keep the read-only snapshot until normal special-buffer retention
or explicit closure removes it; it is useful recovery material.

`:diff-off` closes this comparison through the existing path. A detached clean
disk-snapshot buffer follows the ordinary bounded special-buffer retention
rule and is never allowed to evict a visible or dirty buffer.

## Implementation boundaries

Expected areas of change are:

- `src/buffer.rs` for disk-baseline generations, observation construction or
  classification, convergence, and reload-from-observation checks;
- a new `src/file_monitor.rs` for bounded watch registration, debounce,
  fallback reconciliation, background reads, and typed events;
- `src/app.rs` and the standalone and persistent host loops for service
  ownership and external-change state;
- `src/app/file_workflows.rs` for reload preparation, save preflight,
  `:diff-disk`, and disk-snapshot lifecycle;
- `src/app/input.rs` and `src/app/presentation.rs` for the reload confirmation;
- `src/snapshot.rs`, `src/protocol/`, and `src/ui.rs` for semantic stale state,
  `[STALE]`, and attached-client parity;
- `src/command.rs`, `src/keymap.rs`, and `src/help.rs` for registry-backed
  execution, discovery, and help; and
- the user guide and current UI/keymap reference documents named above.

Key execution, help, and hints must continue to come from the shared registry.
The monitor and core application decide state; no frontend reads paths or file
metadata directly. Buffer text replacements continue to use the existing
buffer and application lifecycle methods rather than direct rope mutation.

## Regression coverage

Add behavior-boundary tests covering at least:

- a clean visible file changed externally becomes `[STALE]` without changing
  its text, selection, undo history, or LSP document;
- a dirty file changed differently becomes `[+] [STALE]`, and every pane
  sharing the buffer shows the same state;
- an external rewrite equal to the buffer converges without clearing usable
  undo history;
- same-size content with a preserved modification time is detected after a
  watcher event, while ordinary save still refuses it without one;
- watcher events produced by Runyte's own save do not make the buffer stale;
- a late observation from before save, reload, save-as, retarget, or close is
  ignored by its generation;
- repeated observations and attachment do not duplicate notifications;
- a hidden open buffer remains marked in the buffer manager;
- persistent mode retains observations made while no TUI is attached;
- deleted, unreadable, and binary replacements preserve the text buffer and
  report the correct limited actions;
- `:reload` on any dirty file opens confirmation, cancellation changes
  nothing, and acceptance reloads and resynchronizes syntax, LSP, and every
  shared view;
- a second disk change while reload confirmation is open prevents the prepared
  reload from applying;
- a known stale ordinary save refuses before trim-on-save changes buffer text;
- `:write!` resolves the stale source only after verified success;
- `Space b d` and `:diff-disk` use the same registered command, create a
  read-only disk buffer on the left, preserve the editable source on the
  right, and show aligned changes through the existing diff snapshots;
- comparison refusal for geometry, size, binary data, deletion, or I/O failure
  leaves pane and buffer state unchanged; and
- core and transported snapshots render `[STALE]` consistently in pane titles,
  the active status line, and the buffer manager.

Use temporary directories for every filesystem test. Tests must not write to
the repository, `.runyte/`, configuration paths, or platform cache paths. Keep
monitor timing deterministic through injectable events or a controllable
worker rather than sleep-based assertions.

Before handoff, run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
