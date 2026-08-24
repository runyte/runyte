---
title: "Control characters in filenames corrupt editable directory projections"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 4213e0c
---

## Resolution

Commit `4213e0c` (`Refuse unrepresentable explorer filenames`) fixed the unsafe projection boundary in `DirectorySnapshot::read_with`. The snapshot reader previously accepted visible UTF-8 filenames containing literal newline and other control characters, after which `render_snapshot` placed those characters into a newline-delimited editable buffer. Row reconciliation could then interpret one filesystem entry as multiple rows with different operations.

Visible entries containing Unicode control characters are now rejected before snapshot entries, row identities, or editable text are constructed. The diagnostic identifies the directory without reproducing the hostile filename. Hidden entries remain outside the snapshot when hidden files are disabled and are validated if the user asks to show them. This deliberately chooses clear refusal over a reversible escaping codec, preserving the existing literal-path editing grammar without introducing ambiguous escape syntax.

`tests/directory_buffer.rs::a_directory_with_a_newline_filename_is_refused_before_rendering` creates a Unix filename containing a newline, verifies that opening the editable explorer fails clearly, and proves that the original file remains while the two misleading split names are not created.

Known limitation: the editable explorer still cannot operate on filenames containing control characters; it reports that boundary instead of providing an escaped editing representation.

## Report

The editable directory explorer could not represent filenames containing newline characters without changing their identity.

Unix permits a filename such as `a\nb`. `DirectorySnapshot` accepted the UTF-8 name, `SnapshotEntry::display_name` rendered it verbatim, and the directory buffer joined entries using newline delimiters. Planning later split the projection at every newline and assigned identities by rendered row. The one original entry was therefore interpreted as a renamed `a` row plus a new `b` row.

As a result, confirming a save of an apparently untouched directory listing could rename the original file and create another file. Carriage returns and other terminal control characters could also forge or confuse the presentation.

Directory rows needed a reversible filename encoding that could not collide with row delimiters. If that was not supportable, opening a directory containing unrepresentable names needed to fail clearly before constructing an editable projection. A Unix regression needed to cover a filename containing a newline and ensure that the explorer did not produce filesystem changes from an unchanged listing.

Relevant code was `src/fs_plan.rs` in `SnapshotEntry::display_name` and `DirectorySnapshot::read_with`, and `src/directory_buffer.rs` in rendering and row reconciliation.
