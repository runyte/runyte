# Explorer settings

The explorer has three presentation controls, and each keeps its state in a
different place. None of them reaches the configuration file, and one of them
does not exist yet.

## Observed behavior

`.` (`ToggleHiddenFiles`) inverts `config.editor.show_hidden_files` in memory
and reloads every clean explorer, but never writes the value back. The change
therefore lasts for the session and is lost on restart, while `Space o o` and
`~/.config/runyte/config.yaml` continue to show the old value.
`App::persisted_config` is not updated either, so the settings page and the
running editor disagree about what is on disk.

Because `editor.show_hidden_files` is also read by the finder, the file
picker, path completion, and search, pressing `.` in an explorer silently
changes the scope of those views as well.

`?` (`ToggleDirectoryDetails`) sets `DirectoryBuffer::details` on the active
buffer alone. A second explorer keeps its own state, and the value is not
configuration, so it is neither shared nor persisted. `DirectoryBuffer::reload`
carries the flag across a reload by hand for that reason.

There is no control over listing order. `read_listed_rows` sorts by path
(`src/fs_plan.rs:318`) and nothing above it can ask for another order.

## Expected behavior

All three are settings, persisted to the configuration file, applied to every
open explorer, and reachable from `Space o o`:

- `editor.show_hidden_files` (existing) — unchanged in meaning; `.` now
  persists it.
- `editor.explorer_details` (new, boolean, default off) — `?` persists it.
- `editor.explorer_sort` (new, default `name`) — one of `name`,
  `name_descending`, `modified`, `modified_descending`, `size`,
  `size_descending`.

Directories group before files in every sort order, including the existing
default, and are ordered among themselves by the same key where it is
meaningful and by name where it is not.

`Tab` in an explorer opens a list of these settings and their values, flat:
one selectable row per value rather than a setting to enter and then a value
to choose. The row matching the current value is marked. `.` and `?` remain
bound as direct toggles; the list is the discoverable path to the same
settings and the only path to the sort order.

The sort order applies to the explorer only. The finder and search continue to
rank by their own relevance.

## Constraints

Listing order must not move into `DirectorySnapshot::read_with`.
`SnapshotEntry` identities are assigned as `EntryId::new(index + 1)` over the
name-sorted rows, and `FsPlan` re-reads the directory the same way to compare
against its baseline. Two reads under different orders would misalign those
identities and a rename could become a delete followed by a create. Sorting
belongs in the projection: `DirectoryBuffer::open` already keeps `row_origins`
beside the text so that reordering rows is not a change of plan.

Sorting needs no additional filesystem access. `EntryFingerprint::capture`
already records `len`, `modified_nanos`, and the entry kind for every listed
entry, whether or not details are shown.

The dirty-buffer guard is not uniform, and the difference is deliberate.
Changing `show_hidden_files` or the sort order re-projects the listing, which
is refused while an explorer holds unsaved edits. Details are a `RowHints`
prefix over the rows already present, so `?` must keep working on a modified
buffer.

Persistence must not be able to withhold the toggle. `persist_setting` fails
when no configuration path was loaded, and `scan_document` rejects a file
using a YAML anchor or a duplicate key — neither has anything to do with the
explorer. A failed save should leave the setting applied for the session and
report that it was not written, rather than refusing the change.
`persist_selected_setting` currently rolls back instead, which is correct for
the settings page and wrong here.

Turning details on or off changes the width of the row prefix, so
`Pane::row_prefix_scroll` must be reset for every pane showing an explorer
rather than for the active buffer alone.

`tests/directory_buffer.rs` presses `.` on an `App` built from
`Config::default()` with no configuration path (`:432`, `:456`, `:487`).
Integration tests link a normally-compiled library, so the `cfg!(test)` guard
that protects unit tests does not apply. Each such test must point
`app.config_path` at a temporary file through `note_loaded_config` before the
toggle can write anything.

## Reproduction

1. Open an explorer and press `.`. Dotfiles appear.
2. Open the finder with `Space f`. Its results now include dotfiles, which no
   action in the finder asked for.
3. Check `~/.config/runyte/config.yaml`. `editor.show_hidden_files` is
   unchanged.
4. Open `Space o o`. The hidden-files row shows the old value.
5. Restart. The explorer hides dotfiles again.
6. Press `?` in one explorer and open a second. The second shows no details.
