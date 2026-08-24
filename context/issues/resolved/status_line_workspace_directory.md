---
title: "The global status line repeated the active file instead of showing the workspace directory"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: cce2f95
---

## Resolution

Commit `cce2f95` (`Show workspace directory in the status line`) changed the
global status snapshot and terminal rendering to present the editor working
directory as explicit workspace context. `App::snapshot` previously copied
the active buffer's display name into `StatusSnapshot::buffer_name`, and
`draw_normal_status` rendered that name even though the active pane title
already owned buffer identity. The global row consequently duplicated the
file path and provided no view of the directory selected at launch or by
`:cd`.

The snapshot now carries `App::working_directory` as
`workspace_directory`, using a lossy presentation string so a non-UTF-8 Unix
path remains renderable without a panic. The status renderer labels the field
`Workspace:` and retains active-buffer dirty and read-only markers plus the
cursor, progress, selection, Git, LSP, and notification fields. When the
directory does not fit, `clip_path_start` measures grapheme clusters in
terminal display cells, retains the identifying path tail, and prefixes it
with the exact ASCII marker `...`. If fewer than three path cells exist, it
omits the path instead of drawing an incomplete prefix.

The bundled local protocol advanced to version 17 because the serialized
status-frame field changed from active-buffer identity to workspace-directory
identity. Rejecting a version-16 peer at the handshake prevents an older
client from interpreting the new value using the previous file-oriented
schema. The README and UI vocabulary now assign buffer identity to pane titles
and workspace context to the global status line.

Tests covering the behavior are:

- `ui::tests::normal_status_names_the_workspace_and_keeps_the_other_status_fields`
  in `src/ui.rs`
- `ui::tests::narrow_status_trims_the_start_of_a_long_unicode_workspace_path`
  in `src/ui.rs`
- `ui::tests::rendered_status_tracks_the_directory_selected_by_cd` in
  `src/ui.rs`
- `snapshot::tests::status_snapshot_handles_a_non_utf8_working_directory_lossily`
  in `src/snapshot.rs`
- `protocol::tests::protocol_version_and_request_bounds_are_explicit` in
  `src/protocol/mod.rs`
- `a_read_only_buffer_is_marked_on_every_surface` in `tests/key_hints.rs`

## Report

The global status line showed the active file path even though file paths were
already displayed in pane titles. Repeating the active file did not provide
the workspace context the global status line was intended to communicate.

The status line needed to show the current workspace directory and make clear
that the value identified the workspace rather than the current file. If the
workspace path was too long for the available status-line width, it needed to
preserve the end of the path and trim the beginning with an exact `...`
prefix.

Relevant areas were status-line rendering in `src/ui.rs`, workspace and
working-directory state in `src/app.rs`, and pane-title path rendering.
