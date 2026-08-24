---
title: "Space f (the file picker) matched only files, never directories"
status: resolved
reported: 2026-08-18
resolved: 2026-08-19
legacy_commit: 26f13c5
---

## Resolution

Commit `26f13c5` (`Show directories in space f, narrow to them with a
trailing /`) makes the file picker's scanner, ranking, and preview all
directory-aware; the accept path needed no change.

`scan_with` in `src/file_picker.rs` already walked every directory to
recurse into it, but only ever pushed files into the batch it emitted —
directories were discovered and then discarded. It now takes an
`include_dirs` flag and, when set, also emits a directory as a `ScanEntry {
path, is_dir }`, carrying the file-type bit `DirEntry::file_type` already
determined during the walk so nothing downstream needs a second `stat` to
tell files and directories apart. `scan_files`/`FileScanner::scan` (used
only by the plain file picker) pass `include_dirs: true`; `scan_content`/
`FileScanner::scan_content` (the fuzzy-grep picker, where "directory" has no
meaning) pass `false` and are otherwise unaffected. `FileEntry` gained the
same `is_dir` bit, and `label()` renders a directory with a trailing `/`.

`FilePicker::rank`/`rank_new_entries` narrow matches to directories whenever
`self.query.ends_with('/')`, fuzzy-matching with the trailing slash trimmed
off first. A second issue turned up in review of that narrowing: `rank` had
been reusing the previous `matches` as its candidate set whenever the caller
asked to narrow rather than rescan, but once a trailing-slash query had
filtered files out of `matches`, typing past the slash (`src/` → `src/main`)
kept narrowing from that already-filtered set and could never recover a file
like `src/main.rs`, independent of whether it actually matched the new
query. `FilePicker` now tracks `directory_only`, the mode the last full
`rank` pass used, and `rank` only narrows from the existing `matches` when
that mode is unchanged from the query edit before it; the moment a query
transitions into or out of directory-only mode, it rescans every entry
instead.

The preview gained a `FilePreview::Directory(Vec<String>)` variant. The
first implementation built it from `fs_plan::DirectorySnapshot`, the same
listing the editable explorer uses, but that snapshot calls
`fs::symlink_metadata` on every entry to carry the fingerprint the
explorer's apply step needs — unbounded synchronous work, and an unbounded
`Vec<String>` cloned into overlay snapshots on every keystroke, for a
directory with hundreds of thousands of entries. `FilePreview::from_directory`
was rewritten as its own bounded `fs::read_dir` walk: it inspects
`entry.file_type()` only for the first `PREVIEW_DIRECTORY_ENTRIES` (512)
entries it keeps, counts everything past that without allocating, and
appends a `"… N more entries not shown"` summary line, mirroring the
existing 64 KiB cap on file previews. `refresh_file_picker_preview` in
`src/app.rs` calls it when the selected entry is a directory, skipping the
live-buffer lookup that only makes sense for files; both preview-rendering
sites (the `OverlayPreview` match in `src/app.rs` and the direct
`draw_picker` render in `src/ui.rs`) gained a `Directory` arm that renders
the listing like a text preview.

Enter opening a directory match in the explorer required no code change:
`open_file` already routed `path.is_dir()` into `retarget_pane_directory`
(the same explorer `Space e` opens), and the picker's Enter handler already
called `open_file` with whatever path was selected — once directories
became selectable entries, that behavior applied for free.

Tests: `nested_ignore_files_negation_hidden_files_and_symlinks_are_respected`
and `directory_scans_inherit_ancestor_ignores_and_reserved_roots_are_rejected`
in `src/file_picker.rs`, updated to assert directories are scanned alongside
files (and that a `dir/**` content-only ignore rule excludes a directory's
contents but not the directory entry itself, matching real gitignore
semantics); `trailing_slash_narrows_matches_to_directories` and
`typing_past_a_trailing_slash_recovers_files_excluded_while_directory_only`
in `src/file_picker.rs`, the latter a regression test for the narrowing bug;
`directory_preview_lists_files_and_subdirectories` and
`directory_preview_bounds_large_listings_and_reports_the_omitted_count` in
`src/file_picker.rs`; `file_picker_lists_directories_and_enter_opens_the_explorer`
in `src/app.rs`, exercising the full picker-to-explorer flow end to end.

Known limitation: the directory preview only ever shows a bounded, sorted
prefix of up to 512 entries plus how many were omitted, not a complete
listing of a very large directory.

## Report

Space f searched only files, not directories. Matching a directory and
pressing Enter did not take the user to the explorer. Ending the typed
string with a slash (`/`) was expected to list only directories. Directories
were expected to show their content (files and subdirs) in the preview.
