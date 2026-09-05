---
title: "Explorer view controls were not settings and offered no listing order"
status: resolved
reported: 2026-09-05
resolved: 2026-09-05
commit: 6967112
---

## Resolution

Fixed by `6967112`, "Make the explorer's three view controls settings".

`toggle_hidden_files` inverted `config.editor.show_hidden_files` in memory
and never called `persist_setting`, so the value was lost on restart and
`App::persisted_config` went on reporting the old one.
`toggle_directory_details` did not touch configuration at all: it set
`DirectoryBuffer::details` on the
active buffer, which is why `DirectoryBuffer::reload` had to carry the flag
across a reload by hand and why two explorers could disagree. Listing order
had no representation anywhere.

All three are now settings on `EditorConfig`. `editor.explorer_sort` is new,
`editor.explorer_details` promotes the per-buffer flag, and
`editor.show_hidden_files` keeps its meaning. `SettingId::EXPLORER` names the
group so that a change from the explorer's keys and a change from the settings
page both reach the open listings. Every explorer change goes through
`App::change_explorer_setting`, which applies the value, writes it, and calls
`refresh_listings`; `DirectoryBuffer::toggle_details` became `set_details`,
since two explorers asked for the same thing must end up alike however they
started, and `ListingView` now carries the three values together through every
entry point that reads a directory.

A failed write does not withhold the change. `persist_setting` fails when no
configuration path was loaded and when `scan_document` refuses a file using a
YAML anchor or a duplicate key, neither of which has anything to do with the
explorer, so `save_explorer_setting` reports the reason on the status line and
the setting still applies to the session. This is a deliberate departure from
`persist_selected_setting`, which rolls back instead — correct for the settings
page, wrong for a view toggle.

Ordering lives in `DirectoryBuffer::open`, not in
`DirectorySnapshot::read_with`. `SnapshotEntry` identities are assigned as
`EntryId::new(index + 1)` over the
snapshot's own name order, and `FsPlan` re-reads the directory in that same
order to compare against its baseline; sorting the read would misalign the two
and could turn a rename into a delete followed by a create. `row_origins`
already carries identity independently of where a row sits, which is what makes
a reordered projection safe. No extra filesystem access was needed:
`EntryFingerprint::capture` already records length and modification time for
every listed entry whether or not details are shown.

`sorted_rows` puts directories before files in every order, then applies the
order's key, then falls back to the row's index. Because the snapshot arrives
in name order that index *is* the name key, so an ascending name sort has
nothing of its own to compare and every other order gets the name as its
tiebreak for free. A size order returns `Ordering::Equal` for a directory, so
directories keep their name order under either direction.

`Tab` reaches all three through the keymap's existing `ContextAction` registry
rather than through a Directory-scope binding. A scoped `Tab` binding would
shadow the global key, which `no_scoped_binding_shadows_a_global_binding`
forbids, and the contextual registry is what that rule exists to point at —
`Tab` already opens it for every other row-oriented view. `h` and `d` toggle
dotfiles and details; `o` runs `choose-explorer-order`, which offers the six
orders as one flat list marking the one in use. This is a deviation from the
report, which asked for a single flat list of all three settings' values: that
would have required a Directory-scope `Tab` binding and so a weakened keymap
invariant, and the two booleans read better as direct menu toggles than as four
rows of `true`/`false`.

The dirty-buffer guard is deliberately not uniform. Hidden files and the order
re-read and re-project the listing, so both are refused while an explorer holds
unsaved edits. Details prefix rows already present and re-read nothing, so `?`
keeps working on a modified explorer.

Tests, in `tests/directory_buffer.rs` unless stated:

- `each_listing_order_sorts_by_its_own_key_with_directories_first` — all six
  orders over a fixture whose name, size, and time orders each differ.
- `a_size_order_leaves_directories_in_name_order`
- `entries_sharing_a_key_keep_their_name_order`
- `a_listing_order_does_not_turn_a_rename_into_a_delete_and_create` — the
  identity guarantee, renaming a row that sits where it does only because of
  the sort.
- `choosing_a_listing_order_saves_it_and_reprojects_the_explorer` — the `Tab`,
  `o`, choose path end to end, including the value reaching `config.yaml`.
- `a_modified_explorer_refuses_a_reprojection_but_still_shows_details`
- `the_dot_key_shows_and_hides_dotfiles_in_the_explorer` and
  `question_mark_toggles_aligned_file_details_without_editing_the_listing` —
  both now point `app.config_path` at a temporary file and assert the written
  value.
- `a_toggle_that_cannot_be_saved_still_changes_the_listing`
- `every_setting_writes_and_reads_back_the_field_its_descriptor_names` and
  `only_the_enumerated_setting_types_offer_values_to_choose_from`, in
  `tests/settings_registry.rs`, extend to `SettingType::ExplorerSort`.

Known limitation: the report asked that `.` stop changing the Finder's,
completion's, and workspace search's scope, and then withdrew that in favour of
consistency once all three controls became saved settings. `.` therefore still
moves those views' scope, and now does so durably rather than for the session.
Only `editor.explorer_sort` is confined to the explorer.

## Report

The explorer has three presentation controls, and each kept its state in a
different place. None of them reached the configuration file, and one of them
did not exist.

`.` (`ToggleHiddenFiles`) inverted `config.editor.show_hidden_files` in memory
and reloaded every clean explorer, but never wrote the value back. The change
therefore lasted for the session and was lost on restart, while `Space o o` and
`~/.config/runyte/config.yaml` continued to show the old value.
`App::persisted_config` was not updated either, so the settings page and the
running editor disagreed about what was on disk.

Because `editor.show_hidden_files` is also read by the finder, the file picker,
path completion, and search, pressing `.` in an explorer silently changed the
scope of those views as well.

`?` (`ToggleDirectoryDetails`) set `DirectoryBuffer::details` on the active
buffer alone. A second explorer kept its own state, and the value was not
configuration, so it was neither shared nor persisted.
`DirectoryBuffer::reload` carried the flag across a reload by hand for that
reason.

There was no control over listing order. `read_listed_rows` sorted by path and
nothing above it could ask for another order.

Expected: all three are settings, persisted, applied to every open explorer,
and reachable from `Space o o` — `editor.show_hidden_files` unchanged in
meaning, a new boolean `editor.explorer_details`, and a new
`editor.explorer_sort` of `name`, `name_descending`, `modified`,
`modified_descending`, `size`, or `size_descending`. Directories group before
files in every order, ordered among themselves by the same key where it is
meaningful and by name where it is not. `Tab` in an explorer offers these
settings and their values, with the value in force marked; `.` and `?` remain
bound as direct toggles. The sort order applies to the explorer only, the
finder and search continuing to rank by their own relevance.

The report left the shape of the `Tab` list undecided between one flat list of
every value and a menu of the three settings; see the deviation recorded above.

Reproduction:

1. Open an explorer and press `.`. Dotfiles appear.
2. Open the finder with `Space f`. Its results now include dotfiles, which no
   action in the finder asked for.
3. Check `~/.config/runyte/config.yaml`. `editor.show_hidden_files` is
   unchanged.
4. Open `Space o o`. The hidden-files row shows the old value.
5. Restart. The explorer hides dotfiles again.
6. Press `?` in one explorer and open a second. The second shows no details.
