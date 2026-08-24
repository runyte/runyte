---
title: "No workspace is ever reported as clean, because the scratchpad counts as unsaved work"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: be430a0
---

## Resolution

Fixed in `be430a0` "Leave the scratchpad out of what makes a workspace
unsaved".

Nothing was wrong with the count itself: every place that asked whether a
workspace held unsaved work asked the same question, `dirty && !closed`, and
`Buffer::dirty` is honest about what it measures. The problem was that a
scratch buffer can never answer no. `Buffer::scratch` starts with
`saved_text: Some(Text::new())`, so `update_dirty` compares the text against
an empty baseline and one keystroke is enough; and it has no path, so nothing
routes it back through `mark_saved` — `save` needs a filename, and `:write`
turns it into a file buffer rather than saving it where it is. Emptying it
again was the only way back to clean.

That answer reached further than the listing. The host refuses `Shutdown`
while the count is above zero, and `may_retire_idle` waits on the same
predicate, so a scratchpad someone had typed a note into held its workspace
open past `workspace.idle_retirement_minutes` and refused `:workspace-stop`
with "workspace has 1 unsaved buffer".

`Buffer::holds_unsaved_work` is the single predicate now: dirty, and not
`BufferKind::Scratch`. It is deliberately narrower than `Buffer::dirty`, which
keeps its meaning everywhere it already had one — the pane's `[+]`, the status
line, the refusal to close a buffer, the guard on partial staging, and the
confirmation a standalone quit asks for. The distinction it draws is whether
the person could keep this text by saving where they are: for a scratchpad
they could not, so a host that stops or retires owes them nothing it can
deliver.

`WorkspaceHost::unsaved_buffers` is the single count above that predicate, and
the shutdown refusal, `may_retire_idle`, and the health response all read it.
The host reports the number itself rather than clients counting a buffer list,
which is a change of shape rather than of policy: `inspect_endpoint` used to
send `Health` and then `ListBuffers` and filter the result, so the listing
held its own opinion about a workspace's cleanliness and could in principle
disagree with the host that owns the refusal. It now reads `unsaved_buffers`
straight off the health response and no longer asks for the buffers at all,
which also halves the round trips behind every row of `--list-workspaces` and
`:wls`. Protocol `VERSION` is 19 for the added field; a host of the previous
build is reported as `running (protocol 18)` by the rule the incompatible-host
work already established.

The count now means unsaved rather than dirty, so the wording follows it:
`WorkspaceRow::dirty_buffers` is `unsaved_buffers`, the picker row reads
`unsaved N`, and the `--list-workspaces` column is `UNSAVED`.

Tests:

- `an_edited_scratch_buffer_is_dirty_but_holds_no_unsaved_work` in
  `src/buffer.rs` pins both halves of the distinction, and that emptying a
  scratchpad returns it to its baseline.
- `an_edited_scratch_buffer_leaves_the_workspace_clean` in
  `src/workspace/host.rs` asserts the buffer still reports itself dirty over
  the metadata while the workspace counts zero and may retire.
- `idle_retirement_requires_clean_buffers_and_no_pending_wait` in
  `src/workspace/host.rs` now dirties a file buffer, since the scratch buffer
  it used to edit no longer blocks retirement.
- `an_edited_scratchpad_leaves_a_workspace_clean_enough_to_stop` in
  `tests/local_protocol.rs` drives the real binary: it edits the scratchpad
  over the socket, reads `unsaved_buffers: 0` from the health response, and
  watches the host accept `Shutdown` and exit.
- `workspace_switch_requests_are_platform_guarded_persistent_and_preserve_dirty_hosts`
  in `src/app.rs` confirms standalone mode cannot leave its in-process host,
  unsupported platforms report their actual boundary, and a persistent host
  may retain dirty buffers across attachment switches.
- `worktree_view_preserves_path_selection_and_switches_only_in_persistent_mode`
  in `src/app.rs` covers the same policy through the worktree list.

Known limitation: an edited scratchpad is now discarded without a prompt when
a host stops or retires, and a persistent client that detaches leaves no
warning behind either. Nothing preserves scratch text across a host's
lifetime; keeping it means writing it to a path.

## Report

In `:wls`, no workspace was ever reported as clean. Every row showed a
non-zero dirty count even when every file in the project had been saved.

The scratch buffer appeared to be the reason. A host starts with one, it is
dirty as soon as anything is typed into it, and nothing can bring it back to
clean: it has no path, so there is nowhere to save it. The count behind that
column is also what a host consults before it agrees to stop and before it
retires while idle, so a few characters left in a scratchpad kept a workspace
alive indefinitely and made `:workspace-stop` refuse.

The scratchpad should be excluded from that condition: a workspace with
everything saved except the scratchpad should be clean.
