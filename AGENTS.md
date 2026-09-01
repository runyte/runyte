# Runyte agent guide

## Project

Runyte is a fast modal terminal editor written in Rust, inspired by Helix and
built on its selection-first model. The editor is the project; keep it first
when weighing a change.

The keymap is close to Helix but has drifted deliberately, most of all in
search and macros. `context/reference/helix-keymap-v1.md` is the register of
record for what matches and what does not; do not describe Runyte as
Helix-compatible without checking it.

Before making substantial changes, read `README.md`, the relevant part of
`docs/user-guide.md`, and `context/README.md`. Then load only the development
records relevant to the task:

- `context/plans/README.md` explains the plan lifecycle and routes to the
  retained architecture records.
- `context/reference/helix-keymap-v1.md` is required when changing editor
  commands, bindings, help, or key hints.
- `context/reference/ui-vocabulary.md` is required when changing buffers,
  panes, lists, pickers, prompts, overlays, or frontend snapshots.
- `context/reference/terminal-compatibility-v1.md` is required when changing
  terminal emulation or PTY behavior.
- `context/reference/startup-performance.md` records measured startup and idle
  cost over time. Consult it when changing startup ordering, document loading,
  syntax parsing, or anything that runs on a timer; the harness that produces
  it is `benchmarks/`.
- `context/issues/` contains open work, `context/issues/deferred/` contains
  confirmed problems awaiting a broader design decision, and
  `context/issues/resolved/` contains searchable diagnoses and regression
  coverage for past fixes.

Historical plans explain decisions but are not a second source of current
behavior. The source, user guide, and current reference documents take
precedence where later work refined a completed plan.

## Terminology

- A **workspace** is the project-root editor scope used in both standalone and
  persistent modes.
- A **persistent session** is the durable local host attachment and retained
  editor state associated with one workspace.
- A **terminal session** is one integrated PTY and child process owned by the
  editor. Use `persistent session` or `terminal session` in prose whenever the
  shorter word would be ambiguous.
- User-facing lifecycle commands use the `session` namespace. Keep workspace
  terminology for project roots, workspace identity and selectors,
  `workspace.mode`, `WorkspaceHost`, and workspace switching.

## Storage boundary

Keep repository development context and runtime workspace state separate:

- `context/` is Git-tracked development context: plans, references, issues,
  and durable project decisions belong here.
- `.runyte/` is Git-ignored editor runtime state used by the optional
  persistent session host and its local transport/lock files.
- Never put runtime state under `context/`.
- Never put credentials, secrets, private model reasoning, or unrestricted
  tool output in tracked context.

`AGENTS.md` is the canonical repository guidance. `CLAUDE.md` points to this
file; do not maintain a second copy.

## Issues

Tracked issues live under `context/issues/`, one Markdown file per issue named
after its topic in `snake_case`: `x_behavior.md`,
`light_and_dark_themes.md`. Files directly in that directory are open; the
ones under `resolved/` are done.

Files under `deferred/` are confirmed problems, but they are also **not open
implementation issues**. Their existing analysis explains why a local fix was
not safe or sufficient. Do not implement or move one in response to a broad
request such as "resolve all remaining issues"; resume it only when the user
explicitly authorizes the deferred work and any required architecture or
design decision has been made.

An open issue is plain prose with no frontmatter or fixed shape. Normalize it
before committing: describe the observed behavior, expected behavior,
constraints, and reproduction in neutral technical language. Preserve exact
keys, examples, errors, and code blocks, but remove conversational requests,
first-person framing, personal paths, and opinions about the implementation
process. An empty file is a placeholder whose report has not been written yet;
ask for it instead of guessing.

Resolving an issue takes two commits:

1. The change itself, with its tests and any `README.md` or
   `context/reference/` updates.
2. A follow-up that moves the file into `context/issues/resolved/` under the
   same name and rewrites it. It is separate because the resolved file
   records the hash of the commit above, which only exists once that commit
   has landed.

A resolved file is frontmatter, then `## Resolution`, then `## Report`:

```markdown
---
title: "One line stating the problem, not the fix"
status: resolved
reported: YYYY-MM-DD
resolved: YYYY-MM-DD
commit: <short hash of the commit that fixed it>
---
```

Resolved records imported at the public-history boundary use
`legacy_commit:` instead. That identifier is provenance from private
development history and does not resolve in the public Git graph. Do not add
new legacy identifiers; fixes made in public history use `commit:` as shown
above.

`## Resolution` opens by naming that commit and its subject, then explains
the diagnosis rather than listing the diff: which function was wrong and
what it was doing instead, what was added and why it is shaped that way, and
any deliberate deviation from Helix or from what the report asked for. Close
it with the tests that cover the behavior, each named with the file it lives
in, so the next agent can run them. Put anything the fix knowingly does not
handle in a `Known limitation:` paragraph at the end.

`## Report` is a neutral account of the problem rather than the instruction
that produced the fix. Keep every technical detail from the open issue — the
exact keys, observed and expected behavior, and any example, diagram, or code
block. Where the report left something undecided, say so plainly instead of
dropping it.

A later fix to an already-resolved issue edits the resolved file in the same
commit as the code, and leaves `commit:` or `legacy_commit:` pointing at the
original implementation.

## Releasing

When asked to bump the version and publish to crates.io, follow
`context/reference/releasing.md` exactly. It is the register of record for the
version scheme, the branch a release is cut from, the publish flags, tagging,
and the order of the push. Do not infer any of those from the commit history.

## Architecture

- `src/app.rs`: editor state, shared application types, and startup coordination.
- `src/app/`: editor-level workflow coordination for commands, key dispatch,
  panes, editing, Git, persistent sessions, terminals, search, files, pickers,
  settings, syntax, and language services. Lower-level feature ownership stays
  in the dedicated modules listed below.
- `src/buffer.rs`: rope-backed buffers, file I/O, and transactional undo.
- `src/text.rs`: rope storage, character offsets, and transactions. Every
  buffer mutation goes through a transaction; nothing writes text directly.
- `src/selection.rs`: normalized multi-range selections over offsets.
- `src/syntax/`: tree-sitter highlighting. The only module aware of
  `tree-house`; everything above it sees `Scope` values and character offsets.
  Grammars are statically linked, not loaded from disk.
- `src/lsp/`: language server client. The only module aware of `lsp-types`.
  All JSON-RPC lives in one Tokio task; the editor holds a non-blocking
  handle and drains events from the main loop, so no language server can
  stall rendering or input.
- `src/headless.rs`: a frontend-independent, test-oriented facade over semantic
  editor commands, transactions, text, selections, and snapshots. It is not an
  RPC or plugin contract.
- `src/snapshot.rs`: owned, presentation-neutral editor and overlay snapshots.
  Frontends render these semantic values without inspecting live editor state.
- `src/protocol/`: private, versioned, bounded DTOs used only by bundled local
  clients. Core workspace values do not serialize themselves.
- `src/picker.rs`: the shared filterable result list behind symbol,
  reference, diagnostic, and code-action pickers.
- `src/file_picker.rs`: the ignore-aware background file scan, its fuzzy
  matcher, and file previews. It never invokes `git`, `fd`, `find`, or an
  external fuzzy finder.
- `src/finder.rs`: the in-memory half of the project finder. Open buffers and
  terminals are already editor state, so this module ranks them and merges
  their scores into the file scanner's list rather than walking the
  filesystem for them. Bare `/` opens the result; buffer search lives on `s`
  and `S`.
- `src/command.rs`: editor command identities and metadata.
- `src/content_alignment.rs`: where a generated read-only page's content sits
  in the pane showing it, as an indent and a row offset recomputed from live
  geometry. Presentation only: no offset in the buffer moves, so a row and a
  column mean the same thing at every pane size. Knows nothing about buffers,
  panes, or drawing.
- `src/hash.rs`: small stable SHA-256 utility used by core persistence and
  identity code.
- `src/path_safety.rs`: canonical project-path containment checks shared by
  editor integrations.
- `src/diff.rs`: the one line diff. Two slices of lines in, a run-by-run
  correspondence out, plus the aligned row space in which matching lines sit
  level. Knows nothing about Git, buffers, or panes; the Git gutter and the
  side-by-side view are both readings of the same alignment, so they cannot
  disagree about what changed.
- `src/diff_view.rs`: a live comparison of two buffers — which panes and
  buffers are the two sides, the cached alignment, and the single aligned row
  both viewports start from. It takes buffer ids rather than paths, so any
  producer of a buffer can be a side.
- `src/directory_buffer.rs`: editable directory projections and hidden entry
  identities.
- `src/directory_listing.rs`: recently read directory listings, kept so that
  completing a path does not re-read the same directory for every keystroke
  and every redraw. A listing is reused only while the directory's own
  modification time says nothing has changed in it. Knows nothing about
  buffers, panes, or what makes a name worth offering.
- `src/fs_plan.rs`: confirmed filesystem plans, conflict detection, trash,
  and cycle-safe application.
- `src/external_open.rs`: binary detection, the platform cache of programs
  chosen for binary files, and the detached spawn that hands one over. Knows
  nothing about buffers or drawing.
- `src/git/`: the only module that runs `git`. Argument vectors, never a
  shell; bounded output; structured errors. `tracker.rs` caches each open
  file's staged text so line marks are recomputed in memory rather than by a
  process per keystroke.
- `src/terminal/`: live terminal session ownership, PTY integration, escape
  parsing, emulation, bounded scrollback, and presentation cells. A terminal
  is pane content, not a buffer, and nothing above this module handles escape
  sequences.
- `src/keymap.rs`: the single declarative source of truth for key execution.
- `src/key_hints.rs`: presentation-neutral key-discovery state.
- `src/help.rs`: the hand-written overview each view opens its help window
  with, chosen from the mode and the binding scope. The key table below it
  still comes from the registry; only the prose lives here.
- `src/jump_labels.rs`: two-character `goto-word` labels and the narrowing
  their keystrokes perform. Knows nothing about drawing or about what makes
  an offset worth labelling.
- `src/ui.rs`: Ratatui rendering.
- `src/workspace/`: workspace identity and state plus the optional persistent
  session host, bounded local protocol, attachment transport, and lifecycle.
- `src/main.rs`: CLI, Crossterm lifecycle, and event loop.

Key dispatch, help, and hints must continue to read from the same keymap
registry.

## Working conventions

- Preserve unrelated user changes and inspect `git status` before editing.
- Keep changes focused and add tests at the behavior boundary being changed.
- Use temporary directories for storage tests; tests must not write into the
  repository's `context/` or `.runyte/`, nor into the person's configuration
  or platform cache directories.
  `external_open::cache_root` returns `None` under `cfg!(test)`, which protects
  unit tests inside `src/` but **not** integration tests under `tests/`: those
  link a normally-compiled library and can reach the real cache directory. A
  test that switches themes must point `app.config_path` at a temporary file
  through `note_loaded_config`; one that answers an "open with" prompt must
  point `app.programs` at a temporary directory, as `tests/key_hints.rs`
  already does. Keep environment-derived paths injectable for that reason.
- Never run a file a test wrote. Writing an executable leaves a descriptor
  open, a concurrent fork elsewhere in the binary inherits it, and the exec is
  then refused with `ETXTBSY` on a loaded machine and nowhere else. Sleeping,
  syncing, retrying, or renaming changes the odds rather than the ownership.
  `src/fixtures/stand-in` is the checked-in executable for this: link to it
  under whatever name the test needs and put the behavior beside the link in a
  `<program>.behavior` data file, which it sources.
- Keep source files MPL-2.0 compatible and retain existing SPDX headers.

Before handing off a Rust change, run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
