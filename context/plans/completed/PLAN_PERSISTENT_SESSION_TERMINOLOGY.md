# Persistent session terminology

Status: completed

## Decision

Runyte uses two related but distinct concepts:

- A **workspace** is the project-root editor scope used by both standalone and
  persistent modes.
- A **persistent session** is the durable local host attachment and retained
  editor state associated with one workspace.

Runyte already calls each integrated PTY a terminal session. User-facing prose
must therefore say `persistent session` or `terminal session` when the shorter
word would be ambiguous. The command line and colon surface use the concise
`session` namespace because their operations cannot address terminal sessions.

This was a clean replacement: the superseded `--workspace-*`, `--wls`,
`--wst`, `:workspace-*`, `:wls`, `:wst`, and `:wat` spellings were removed
rather than retained as compatibility aliases.

## Public command surface

The command line becomes:

```text
-a, --persistent
    --serve
-l, --session-list
    --session-start [WORKSPACE]
-s, --session-stop [WORKSPACE]
    --session-stop-all
    --session-clear-all
    --session-restart [WORKSPACE]
    --session-rename WORKSPACE NAME
-f, --force
```

`WORKSPACE` remains the correct operand name: it is a full displayed ID, an
unambiguous ID prefix, an exact persistent-session name, or a project
directory. Omitting it from start, stop, or restart selects the workspace
discovered from the shell's current directory. Start is idempotent: it starts a
detached persistent session when none is running and succeeds when the selected
session is already running.

The editor surface becomes:

```text
:session-list                         alias: :sl
:session-attach WORKSPACE
:session-start [WORKSPACE]
:session-stop [WORKSPACE]
:session-rename WORKSPACE NAME
:detach
```

`Space W` continues to open the same list and its coherent Tab action menu.
The list is available in standalone mode. Attachment, start, and stop retain
their existing persistent-mode availability boundary; rename remains a catalog
operation available wherever the session list is available. `:detach` remains
persistent-only and has no force form.

`--session-clear-all` retains the existing narrow behavior: it clears only
stopped recent-session records, never project directories, running sessions,
or editor data. `--force` remains valid only with stop, stop-all, and restart.

## Source boundary

Rename internal identities that specifically encode the public persistent
session lifecycle:

- command-line launch modes;
- colon command variants and command specifications;
- top-level CLI lifecycle functions and user-facing status/error text;
- session-manager presentation titles and action descriptions;
- focused tests and fixtures for those public spellings.

Keep workspace names where the value is genuinely a project/editor scope:

- `project_root`, workspace discovery and identity, and workspace selectors;
- `workspace.mode` and other workspace configuration;
- `WorkspaceHost`, which is also the standalone editor owner;
- `WorkspaceRow`, the catalog, recents, endpoint identity, and workspace
  switching;
- `src/workspace/` and protocol messages whose meaning is technical host or
  project-root ownership rather than public command terminology.

The terminology change applied to current documentation, reference material,
source comments, generated help, and focused tests. Historical reports retain
the wording needed to explain the behavior they originally diagnosed.

## Parsing

The command-line parser consumes exactly two values for
`--session-rename WORKSPACE NAME`. The colon parser gains a typed two-part
persistent-session rename argument: the first token is a workspace selector
and the non-empty remainder is the new name. A selector containing whitespace
must be quoted; the name may contain whitespace. Both paths produce one owned
invocation parameter rather than reparsing command text in `App`.

## Verification

Tests must cover:

- every new long and short CLI spelling;
- rejection of every removed workspace spelling and old abbreviation;
- optional current-workspace selection for start, stop, and restart;
- idempotent CLI start for both stopped and already-running sessions;
- session rename by workspace ID, name, and directory;
- colon parsing, help, completion, availability, and execution for every new
  command;
- session-list presentation and action behavior in standalone and persistent
  modes;
- existing protected-state refusal and force behavior under the new names;
- packaging help containing only the new public surface.

Before completion run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
