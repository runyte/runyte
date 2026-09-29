# Editing commands and action bindings

Approved and completed 2026-09-29. Implemented as independently reviewable
changes, with action bindings separate from the default `X` change.

## Line extension — issue #4

Add `extend-line-above` and `extend-line-below` semantic commands. They first
expand a partial selection to full lines, then grow at the requested outer
edge, independently of its direction. `X` becomes `extend-line-above`; `x`
retains `select-line`. Retain `select-line-up` for custom bindings. Preserve
Runyte's inclusive character offsets and transient whole-line behavior for
yank/delete/paste. Counts, multiple selections, empty lines, document edges,
selection undo/redo, and terminal review must agree. The original selection
undo work in issue #4 is already complete; this is its follow-up request.

## Action bindings

Keep `keys.leader`, `keys.window`, and `keys.rebind` unchanged. Add `keys.bind`
with independent `normal`, `select`, `insert`, and `replace` mode mappings:

```yaml
keys:
  leader: Ctrl-x
  rebind:
    Space g: Leader G
  bind:
    normal:
      X: extend-line-above
      Alt-x: select-line-up
      F6: [select-all, yank]
      F7: { command: pipe, argument: sort }
      F8: null
    select:
      X: extend-line-above
    insert:
      Ctrl-s: save
```

Compile action assignments after simple remapping. Left sides are final key
sequences and support `Leader`/`Window`. An exact assignment replaces that
mode's global binding; null removes it. Reject prefix collisions and scoped
shadowing rather than deleting groups or taking ownership of prompts, lists,
terminal input, or native confirmations. Reject grammar-reserved keys and
unreachable bindings. Validate all assignments together, independent of YAML
order; recover invalid rules non-fatally without dropping valid remappings.

Resolve existing semantic command names rather than default keys. Editor
actions use their canonical metadata names; colon commands reuse their existing
parser for optional/required arguments. Exclude internal, retired, unsupported,
and context-only actions from general bindings. Validate editor actions against
their supported modes. Commands without defaults are eligible when explicitly
declared. Keep plugin binding ownership in `plugins[].bindings` for this change.

Sequences run directly, in order, without recursively resolving keys. Only
synchronous actions that do not transfer input ownership may precede another
action. Interactive, asynchronous, mode-entry, and character-taking actions
must be last; runtime errors, unavailability, confirmations, prompts, or async
requests stop execution. Completed effects are not rolled back. Single actions
retain existing count behavior; counts on multiple actions are rejected before
execution. Selection history groups the physical gesture. Ordinary text undo
and Insert-mode checkpoints retain existing command semantics.

Use bounded compilation: 256 assignments, 8 keys per sequence, 16 actions per
binding, bounded argument text. Store configured actions in the same registry
used by dispatch, help and hints; show the entire action sequence. Authored key
hints must not advertise an overwritten key as still invoking its old action.
Publish an action reference from the same eligibility metadata. Configuration
reload adopts valid bindings and resets pending input using existing lifecycle.

## Open-line indentation — issue #8

`o` and `O` copy the current row's exact leading spaces/tabs into the new row,
placing each caret after that prefix. This is baseline indentation preservation,
including when smart newline is disabled; it does not add Markdown markers or
infer a new block level. Preserve line endings and one insertion per distinct
selected row. Keep the opening edit and subsequent Insert session in one undo
group. Test blank/whitespace rows, Unicode, mixed indentation, CRLF, EOF,
multiple selections and undo/redo.

## Optional auto-closing — issue #9

Add `editor.auto_close: false`, exposed as a live boolean in `Space o o` with
ordinary preview, rollback, persistence, and reload behavior. Cover `()`, `[]`,
`{}`, single and double quotes. Apply to typed Insert-mode characters only;
paste, replacement, prompts and terminal input remain literal. Pair an opener
before whitespace, a closing delimiter, or EOF. Quotes also require a boundary
before the caret and must not pair escaped quotes or apostrophes within words.
Skip an existing matching closer and delete both halves of an empty pair with
Backspace. Use bounded local context without a syntax-tree dependency. Every
caret decides from pre-edit text; apply one transaction and map resulting
carets through it. Test the default-off behavior, multiple carets, nesting,
quotes/escapes, Unicode, undo, literal paste and settings persistence.

## Validation and records

Add behavior tests at semantic execution, physical key dispatch, compiler,
settings and terminal-review boundaries as applicable. Run `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and canonical
`cargo llvm-cov --locked --workspace`; retain the 89% floor. Native macOS and
Windows validation remains CI-owned when unavailable locally. Update the user
guide, configuration example, keymap register, and teaching prose with each
change. Commit fixes separately; follow each tracked issue fix with its resolved
record citing the implementation commit. Do not publish GitHub comments or close
remote issues as part of local implementation.

## Implementation notes

`src/keymap/actions.rs` owns bounded action parsing and assignment rollback.
Editor mode admission comes from the implemented global command registry plus
explicitly admitted commands without default keys. A conservative synchronous
command inventory controls sequence continuation. The same metadata generates
`:help key-actions`; it is not a separately maintained list of command names.

`Binding.actions` contains editor identities or validated `CommandInvocation`
values. Ordinary bindings keep an empty action list. The target remains a
leading identity for existing registry consumers; callers advertising an
individual command use `is_plain_action` so a sequence or a different argument
cannot masquerade as that command. The grammar produces ordered intents and
uses the final identity for completed-action feedback. The application stops
at errors or input/async handoffs while retaining the existing selection-action
boundary around the physical input.

Insert/Replace overrides preserve the original terminal binding in Terminal
scope. Configured editing actions are invisible in terminal input, including
window-prefix suffixes. Lookup, validation and help use the same visibility
predicate. Normal/Select overrides remain available in terminal review.

Alias advertisements are removed when their effective destination changes.
Teaching markers follow remaining bindings separately by mode or label the
action unbound. Help retains complete action lists and arguments; the hint
snapshot does too, while fixed-width hint cells abbreviate long descriptions.

## Implementation commits

- `4c8363f`: outer-edge line extension and the default `X` binding.
  `b95eaa8` corrects first-use and count handling on short rows.
- `3371da0`: exact indentation preservation for `o` and `O`.
- `2248071`: optional bracket and quote auto-closing. `608fbf9` aligns
  settings inventories; `baa3c1a` covers paired Backspace after backslashes.
- `312ef18`: mode-specific action bindings, sequences, generated action
  reference, and teaching-surface integration, separate from the `X` change.

Each newly tracked issue has a separate resolved-record commit after its fix.
The existing configurable-key-bindings resolution was updated with its
action-binding extension while retaining the original implementation hash.

## Final validation

On native `x86_64-unknown-linux-gnu`, Rust 1.97.1 and cargo-llvm-cov 0.9.0:

- `cargo fmt --check` passed.
- `cargo clippy --all-targets -- -D warnings` passed.
- `cargo test` passed 4,207 tests, with 41 ignored.
- `cargo llvm-cov --locked --workspace` passed with 92.01% total line
  coverage (137,309 of 149,236 lines), above the unchanged 89% floor.

Native macOS and Windows checks were unavailable locally and remain CI-owned.
No remote issue changes or publishing were performed.
