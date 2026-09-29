# Directory tree sidebar

Status: completed; implementation authorized 2026-09-28

## User interaction

Add one workspace-owned directory tree at the left of the editor area.
`Space d t` toggles its visibility. `Space d d` shows and focuses it, expands
the active file's ancestors, and selects that file. A rendered Markdown page
uses its source path. A directory buffer reveals its directory; a pathless
buffer or terminal selects the workspace root. An outside-workspace document
leaves the root selected and reports that it cannot be revealed here.

The root is the stable project root, independent of `:cd`. Showing the tree
does not create a pane or buffer. Its state survives hiding it. Existing
explorer commands retain their editable-directory behavior.

The tree has a selected path, expansion set, scroll position and its own focus.
The active ordinary pane remains recorded while the sidebar has input focus.
Use `j`/`k` and arrows for rows, Home/End and page motions for larger movement,
`l`/Right to expand or enter an expanded directory's first child, and `h`/Left
to collapse or select the parent. Enter toggles a directory; on a file it opens
that exact path in the last focused ordinary pane and returns focus there.
Escape returns focus to that pane without hiding the sidebar. Register these
commands and keys rather than adding a second hardcoded navigation table.

Remember pane identity, not an index into a changing pane list. If the recorded
pane closes, choose the surviving active pane. An ordinary pane may show a
terminal: opening a file covers its terminal through the existing destination
workflow and retains the terminal session. Never insert keys into a terminal
while the sidebar owns focus. Failed opens keep tree focus and selection.
Opening binary files follows existing external-open behavior.

`Tab` opens the shared contextual action presentation:

| Key | Action | Behavior |
| --- | --- | --- |
| `n` | New file or directory | Prompt relative to the selected directory or selected file's parent; trailing `/` creates a directory. |
| `r` | Rename | Prefill the selected basename; accept one name, not a destination path. |
| `d` | Delete | Stage deletion of the selected entry. |
| `m` | Move | Prompt with path completion; relative paths resolve against the source parent. An existing directory or a trailing separator means move inside it with the existing basename; otherwise the path is the complete destination name. |
| `p` | Review pending changes | Open the existing filesystem confirmation for the entire pending plan. |
| `u` | Undo pending change | Remove the most recently staged action, without touching disk. |
| `c` | Clear pending changes | Clear the pending plan through an explicit discard confirmation. |

The last two actions make mistakes recoverable before application. The root
cannot be renamed, moved or deleted. Context menus show availability reasons.
Prompts retain the captured row/path and plan revision; accepting a prompt
must not act on whichever row a later refresh selected. Escape cancels only
the prompt or menu and restores tree focus.

Staging never changes disk. Mark existing source rows for rename/move/delete;
show the planned destination in their detail. Show synthetic pending-create
rows under their target parents, and a pending-operation count even when an
affected directory is collapsed. Escape from review preserves staged changes.
The confirmation keeps Enter for trash-first application and explicit `P` for
permanent deletion. Hiding the tree preserves pending changes. A normal quit
that would destroy App must protect a pending plan through the existing
unsaved-work refusal/force convention; detach retains it in its host.

## Ownership and layout

Add `src/directory_tree.rs` for tree entries, stable path identities, expansion,
flattening, selection and staged intentions. Keep app coordination in
`src/app/directory_tree.rs`. Filesystem mutation continues to belong to
`src/fs_plan.rs`; tree drawing belongs to snapshot-driven UI code.

Add an optional tree state and explicit sidebar focus to `App`. Avoid assigning
an invented buffer ID or inserting the tree into `Layout`. Existing pane
history, resource lists and buffer closing remain about their existing objects.
Sidebar focus must be consulted before normal editor/terminal dispatch and
before mouse selection, text paste, scrolling, command availability and cursor
presentation. Prompts/confirmations retain higher input priority.

In `prepare_view`, split the editor rectangle into sidebar and pane rectangles
after session-strip geometry is established. Pass the reduced pane rectangle
to the existing `Layout::rectangles` so every split keeps its proportions.
Use a preferred sidebar width of 28 cells, capped at one third of editor width;
below 36 columns suppress the visible sidebar and return input focus to the
pane while retaining requested visibility. Re-expanding the terminal restores
visibility without stealing focus. Bounds must remain valid down to 0x0.
Overlay centering continues to use the complete editor area.

Fullscreen and zen temporarily suppress the sidebar. Retain its visibility
preference and restore it on leaving maximization. Invoking reveal explicitly
leaves maximization so the requested tree can be shown. Toggling off while
maximized updates the preference. Normal pane focus/mouse commands return
input focus to the pane, with existing terminal mode restoration. Keep pane
split geometry and PTY resize behavior derived from the same prepared areas.

Expose a bounded `DirectoryTreeSnapshot` on `EditorSnapshot`: rectangle,
focus, title/root label, visible semantic rows, selected-row identity/index,
indent depth, entry kind, expanded/loading/error state and pending markers.
Snapshot production escapes control characters in labels but never uses
display strings as filesystem identities. Long names truncate by display
width; Unicode must not corrupt hit bounds. Mouse hit testing uses prepared
row identities. Click selects/focuses; double-click is optional, Enter suffices.
Wheel movement in the sidebar must not scroll the underlying pane.

Update `src/protocol/frame.rs` conversions and validation and bump private
`protocol::VERSION` (currently 63). Validate bounded rows, strings, depths,
indexes and geometry; include the tree in full/damage frame equality so an
attached TUI sees selection, expansion, focus and pending changes. Both local
and attached rendering consume the same semantic snapshot. The host's App
retains tree state across detach/reattach. No new on-disk tree persistence is
required; a newly started editor begins with the tree hidden.

## Listing and performance

Read only expanded directories. Use a small bounded background listing worker
or the existing suitable filesystem worker mechanism; do not recursively scan
the workspace on toggle, reveal, startup or redraw. Tag results by requested
root/directory and generation so stale results cannot replace a newer view.
Do not perform filesystem reads in immutable rendering or snapshot conversion.
Keep the previous rows during refresh, preserving the selected path where it
survives and otherwise selecting its nearest surviving ancestor/neighbor.

Directories sort before files, then by name. Honor `editor.show_hidden_files`.
Reveal may temporarily expose a hidden path and its ancestors without saving
a global setting. Do not recurse through directory symlinks; show them as
symlinks. Escaped newline/control names remain one row with their real path
retained. Read failures appear as directory errors that can be retried.
Refresh on explicit tree refresh (`Space r` in tree context), expansion and
successful filesystem application. No new polling timer is required. Pending
baselines remain frozen across browsing refreshes; refreshing must not accept
external changes on behalf of a pending destructive operation.

## One plan across directories

Current `FsPlan` stores one `DirectorySnapshot`, and `FsConfirmation` requires
an explorer buffer ID. Neither represents this feature directly. Extend these
ownership boundaries explicitly; do not create a hidden directory buffer, fake
a recursive root listing, or apply several independently confirmed plans in a
loop.

Add a constructor for explicit staged create/rename/move/delete intentions,
with canonical absolute plan root, captured source fingerprints, and captured
directory baselines for touched source/destination parents. A source capture
must happen when its action is staged, not freshly when Review is pressed.
Existing `FsPlan::build` remains supported for explorer and plugin callers.
The implementation may generalize its expected-directory storage to a list or
add a separate explicit baseline variant; all paths entering the common
operation scheduler must be relative to the one plan root using existing path
normalization rules. Do not rely on directory-local `EntryId` being globally
unique. Source identity is its captured path plus fingerprint.

Before any mutation, revalidate every captured directory and operation source,
then preflight the entire operation set together. This retains cross-directory
collision checking and cycle-safe staging. Reuse exclusive destination
installation, Windows name rules/case behavior, trash backend, rollback and
recovery reporting. A late collision must never overwrite another entry.
Path completion may offer existing outside-root destinations; existing planner
relative-path handling remains responsible for them. Cross-device moves retain
the existing refusal, with no copy/delete fallback.

Normalize repeated actions on one source into one final intention: rename then
move retains the original source and capture; delete supersedes that source's
pending rename/move; deleting a synthetic new entry cancels its creation. Undo
restores the previous intention state. Reject duplicate final targets and
directory self-descendants. For the first implementation, refuse overlapping
existing-directory and descendant operations in one plan with a clear message
to apply them separately. This avoids silently rebasing descendants of a moved
or deleted parent. Support creation beneath directories created in the same
plan through the existing dependency scheduler, or explicitly refuse before
staging if that dependency cannot be represented; never fail only after some
unrelated action already applied.

Replace confirmation's implicit buffer ownership with explicit origin, such
as `Explorer { buffer }`, `DirectoryTree { revision }`, and the existing plugin
ownership path. Keep plugin native-input authorization intact. Tree plans pass
no initiating buffer to `reconcile_applied_filesystem`, preserving dirty
explorers while updating open file paths, language services and Git state.
After success, clear the applied intentions, refresh touched tree directories
and select a surviving destination. After an error with no applied operations,
retain intentions for review/discard. After partial application or retained
recovery artifacts, reconcile from the actual report, invalidate the attempted
pending plan and require fresh staging; never permit replay of applied actions.
Keep the existing notification containing every recovery path.

The feature does not change the existing documented parent/source substitution
race limitations or make whole filesystem plans atomic. The deferred symlink
confinement issue remains deferred.

## Registry and documentation

Add dedicated tree commands and `BindingScope::DirectoryTree`. Ensure its
navigation scope cannot inherit text-mutating commands or terminal input by
accident. Use the registry for the new `Space d` namespace, navigation, Tab
context actions, help, key hints and remapping. Review `BindingScope::ALL`,
configured-scope parsing and effective-scope validation. Keep tree scope out
of special-buffer lifetime classification.

Update README, the user guide, `helix-keymap-v1.md` and `ui-vocabulary.md` with
sidebar ownership, focus, keys, staging, plan cancellation, layout and limits.
Add sidebar as a separate persistent navigation surface in the vocabulary.
Move this plan to `completed/` only after implementation and review acceptance.

## Implementation order and validation

1. Add tree model and lazy listing with bounded state and meaningful model tests.
2. Add explicit multi-directory plan construction and confirmation origin;
   verify staging captures and preflight before integrating UI.
3. Add App ownership, focus, reveal/open behavior and registry commands.
4. Add geometry, snapshot, rendering, mouse routing and private protocol.
5. Add prompts, pending markers, review/apply/undo/clear and reconciliation.
6. Update behavior documentation and run required gates.

Behavior tests should live in `tests/directory_tree.rs`,
`src/app/tests/directory_tree.rs`, existing `tests/fs_plan.rs`, keymap tests and
snapshot/protocol tests as appropriate. Required scenarios:

- Toggle/reveal keeps pane IDs, buffers and split ratios; hidden tree does no
  work; resize/fullscreen/zen restores correctly; tiny geometry never panics.
- Navigation, expanded-state retention, hidden reveal, empty/inaccessible
  directories, symlinks, Unicode/control names and stale worker results.
- Open into the remembered pane after another pane was focused; close that
  pane; terminal-backed destination; dirty existing buffer; failed/binary open.
- Tree typing/paste/mouse wheel does not modify a buffer or send terminal input;
  Escape and pane focus recover the expected mode; overlays own input first.
- Registry hints/help/remapping agree with dispatch and new Tab actions.
- Stage all four actions without disk mutation; pending markers and undo/clear;
  prompt capture; canceled review retains plan; nested new directories; repeated
  source edits; rename/move collisions and explicit overlap refusal.
- A single plan touching two directories is entirely refused before mutation
  when either baseline/source changed. Content edits after staging and after
  review both refuse destructive actions. Directory-local ID collisions cannot
  redirect operations. Exercise cross-directory rename cycles, exclusive late
  collisions, injected trash failure and partial/recovery reports.
- Applying a tree plan retargets open file buffers, keeps dirty explorer text,
  refreshes tree state, and invalidates unsafe replay after partial application.
- Full and damage snapshots round-trip with the sidebar, have bounded protocol
  validation, and produce equivalent local/attached TUI output; host detach
  retains state while canceling transient native input appropriately.

Use fixture-owned temporary storage and existing injected trash/system-opening
ports. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and canonical `cargo llvm-cov --locked --workspace`. Preserve the
89% Linux/macOS line-coverage floor; report platform gates that cannot run on
the local target. Use focused performance evidence for lazy listing and idle
behavior; no startup scan or extra timer is acceptable. Iterate independent
review and fixes until the reviewer reports no remaining findings.
