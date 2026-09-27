# Explorer rejects a new row inside an existing directory

In an editable explorer, adding a new row whose path goes through a directory
that already exists fails at `:w` with a copy error. No filesystem plan opens.
A new row at the explorer's own level works, and so does a new row inside a
directory that does not exist yet.

## Reproduction

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

The result is the same when the new line is opened directly below the
`docs/` row (`gg`, `o`).

Expected: the plan offers `create docs/roadmap.md`, and Enter creates the file
inside the existing `docs/` directory.

## Comparison

In the same listing, these edits produce a correct plan:

- A new row `CHANGELOG.md` at the explorer's level: `create CHANGELOG.md`.
- A new row `plans/roadmap.md`, where `plans/` does not exist: plan shown.
- Editing the existing `notes.md` row to `docs/notes.md`:
  `move notes.md → docs/notes.md`.

## Notes

The message comes from the duplicate-entry check in `src/fs_plan.rs`, which
reports `cannot copy {} inside itself` when a directory entry has a duplicate
row below its own path. The new `docs/roadmap.md` row therefore appears to be
treated as a second row of the existing `docs/` entry, not as a new entry.
Moving an existing file into `docs/` is not affected.

Observed with Runyte 0.3.3 on Linux.
