---
title: "Explorer rejects a new row inside an existing directory"
status: resolved
reported: 2026-09-27
resolved: 2026-09-28
commit: e838d4b
---

## Resolution

Commit `e838d4b` (`Keep new explorer rows separate from existing directory
entries`) fixed identity assignment in `DirectoryBuffer::reconcile`. As a new
row was typed, its temporary `docs/` text matched the existing directory row.
The duplicate-label path and baseline-label recovery assigned that directory's
identity to the new row. When the text became `docs/roadmap.md`, `FsPlan::build`
therefore treated it as a second copy of `docs/` and rejected copying a
directory inside itself.

Reconciliation now inherits a duplicate row's identity only when the
transaction adds a row, as a paste does. Baseline-label recovery also refuses
to assign an identity still carried by an existing row. A newly typed nested
path remains a create, while cut-and-paste restoration, copies, moves, and
creation below a newly added directory retain their plan semantics.

Coverage is in `tests/directory_buffer.rs`:

- `typing_a_new_path_inside_an_existing_directory_creates_a_file` checks both
  reported insertion positions, the create plan, and confirmed `:w`.
- `a_cut_directory_row_restores_its_identity_on_paste` checks that moving a row
  within the listing does not produce a filesystem operation.
- `pasting_an_existing_directory_still_plans_a_copy` checks duplicate-path
  rejection and the copy plan after the pasted row is renamed.
- `editing_an_existing_file_into_a_directory_still_plans_a_move` checks the
  reported move comparison.
- `new_nested_rows_create_the_parent_before_the_file` checks creation order
  and the resulting nested file.

## Report

In an editable explorer, adding a new row whose path went through a directory
that already existed failed at `:w` with a copy error. No filesystem plan
opened. A new row at the explorer's own level worked, as did a new row inside
a directory that did not exist yet.

### Reproduction

Open an explorer (`Space e`) on a directory that contains a `docs/`
subdirectory, for example:

```text
docs/
src/
Cargo.toml
notes.md
```

Add a new line anywhere in the listing, for example with `o` on the
`notes.md` row, containing:

```text
docs/roadmap.md
```

Press Escape, then `:w`.

Observed: no plan opens, and the message line reports

```text
:w (Write the active buffer · failed: cannot copy docs inside itself)
```

The result was the same when the new line was opened directly below the
`docs/` row (`gg`, `o`).

Expected: the plan offers `create docs/roadmap.md`, and Enter creates the file
inside the existing `docs/` directory.

### Comparison

In the same listing, these edits produced a correct plan:

- A new row `CHANGELOG.md` at the explorer's level: `create CHANGELOG.md`.
- A new row `plans/roadmap.md`, where `plans/` does not exist: plan shown.
- Editing the existing `notes.md` row to `docs/notes.md`:
  `move notes.md → docs/notes.md`.

### Notes

The message came from the duplicate-entry check in `src/fs_plan.rs`, which
reports `cannot copy {} inside itself` when a directory entry has a duplicate
row below its own path. The new `docs/roadmap.md` row therefore appeared to be
treated as a second row of the existing `docs/` entry, rather than a new entry.
Moving an existing file into `docs/` was unaffected.

Observed with Runyte 0.3.3 on Linux.
