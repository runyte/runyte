---
title: "Special files can block Finder scans and previews"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 16952e2
---

## Resolution

Commit `16952e2` (`fix(finder): bound reads and reject special files in workers`).

Finder ignore rules, content searches, and previews now open descriptor-validated regular files. Content and ignore reads enforce a 4 MiB byte limit after opening, so replacement or growth cannot bypass a pathname metadata check. Snippet reads are bounded as well. Invalid or oversized ignore files count as skipped sources, preserving ordinary rules and regular-file symlink support.

Coverage: finder_special_files_return_without_waiting_for_a_writer in tests/finder_special_files.rs owns bounded subprocess cases for ignore scans, previews, and snippets, including FIFO aliases, a special device, and a regular symlink. Its ignore case timed out before the fix. finder_text_reads_enforce_byte_limits_and_skip_oversized_ignore_files in src/file_picker.rs covers exact Unicode byte limits and oversized metadata/content. All 52 selected file-picker tests and the integration parent pass; the owned child fixture is ignored outside its parent.

The subprocess fixture compares the returned ordinary-file path with its
canonical path, matching `scan_with`'s root canonicalization. On macOS, a
temporary root spelled through `/var` resolves under `/private/var`; comparing
the original spelling caused a test failure after the special-file checks
had passed. The macOS CI jobs set `TMPDIR=/private/tmp`, whose spelling is
already canonical, so that configuration did not expose the mismatch.

## Report

Finder workers read `.gitignore` and `.ignore` with `fs::read_to_string`
without validating their file type. An ignore file that is a named pipe with
no writer blocks the entire name or content scan indefinitely. Cancelling
the finder cannot interrupt that blocking open. The shared preview worker
also opens any path as an ordinary file, so a previously discovered file
replaced with a named pipe stalls later previews. Content reads check a
pathname's size before reopening it without a read limit; replacement or
growth can bypass the intended 4 MiB limit.

Reproduce the ignore case by creating a temporary project with an ordinary
text file and `.ignore` as a FIFO, then starting an ignore-aware Finder scan.
No terminal scan result arrives. Previewing a FIFO directly through
`FilePreview::from_path` or `FilePreview::snippet_from_path` also waits for a
writer instead of reporting an unreadable source.

Open regular files through a descriptor-validated, nonblocking boundary and
apply content limits to the bytes read, including snippet previews. Invalid
ignore files should count as skipped inputs while ordinary ignore rules and
regular-file symlink behavior remain supported. Bound ignore-file input to
4 MiB before parsing so malformed or growing metadata cannot consume an
unbounded worker allocation. Tests should run FIFO cases inside a bounded
subprocess and keep all filesystem/configuration paths in temporary storage.
