<!-- SPDX-License-Identifier: MPL-2.0 -->
# Composable key bindings: actions, programs, macros and presets

Tracking issue: [#132](https://github.com/runyte/runyte/issues/132).
Status: active, design approved 2026-11-26. Breaking change, released as 0.5.0.
Related: [#123](https://github.com/runyte/runyte/issues/123) (modes per
window type), [#124](https://github.com/runyte/runyte/issues/124) and
[#127](https://github.com/runyte/runyte/issues/127) (atomic splits),
[PR #129](https://github.com/runyte/runyte/pull/129).

## Problem

Custom bindings are hard to build because the parts they are built from are
inconsistent. #124 and #129 split `yank`, `paste-after` and `*` into atomic
actions and kept the old behaviour under new names. Repeating that for each
remaining command would double the action list without making it easier to
understand. The causes are structural:

1. **Six configuration surfaces with different rules.** `keys.bind` works per
   mode and accepts lists and arguments. `keys.scopes` works per view but
   takes one action, without arguments or lists. `keys.ui` has its own action
   names per input owner. `keys.pointer`, `keys.rebind` and
   `plugins[].bindings` are separate again.
2. **Built-in bindings name one action.** `Binding` already has an `actions`
   field (`src/keymap.rs`), but only configured bindings fill it. A default key
   that does two things therefore needs a bundled action, such as
   `yank-collapse-normal-mode` beside `yank`.
3. **Actions do not describe themselves.** The modes in which an action is
   configurable are inferred from where its default key is bound, plus a
   hand-kept exception list (`scan_editor_modes` in `src/keymap/actions.rs`).
   Whether it may continue a list is a second hand-kept list (`can_continue`).
   Whether it accepts a count is a third flag (`accepts_count`).
4. **Per-view behaviour is hidden.** A terminal intercepts `yank`,
   `paste-after-or-replace`, `paste-before`, `search-next` and others and does
   something different (`src/app/terminal_workflows.rs`). Help does not say so.
5. **Many actions change the mode or collapse the selection as a side
   effect.** `paste-before`, `toggle-case`, `replace-char`, `delete-selection`,
   `select-all` and `collapse-selection` all set the mode. `search-next` always
   narrows to one match and enters Select mode. These side effects are why
   lists cannot be assembled freely.
6. **Macros can only be recorded.** They are held in memory as recorded input,
   cannot be defined in configuration, cannot be bound to a key and are listed
   only by register.
7. **Dead grammar code.** The Vim grammar was removed from production in
   August 2026 (`context/issues/resolved/remove_vim_mode.md`). Its interpreter
   is compiled only for tests, and its handlers (`RangeIntent::Vim*`,
   `apply_vim_motion`, `apply_vim_operator`, `find_search`,
   `search_selection_direction`, `SearchForward`, `SearchBackward`,
   `GrammarKind::Vim`) remain compiled but unreachable, with about 146
   references across 10 files outside `tests/`.

There are 367 editor actions. Part of that count is bundled duplicates; part
is actions whose behaviour depends on the view without saying so.

## Goals

- A user can see exactly what any key does: the actions it runs, in order, and
  where the binding comes from.
- A user can list and search every action, with what it does in each view and
  mode, whether it can be followed by another action, and its count and
  argument.
- One binding format for every view and mode, used by the built-in keymap and
  by user configuration alike.
- Atomic actions only, except where an action makes a decision that a program
  cannot express. Each such action is documented as a decision.
- A clear divide between actions (semantic operations) and macros (replayed
  key input), with macros definable and bindable in configuration.
- A model expressive enough that both a Helix-style and a Vim-style editing
  grammar can be written as configuration. The Vim-style preset is an
  acceptance test of the model.

## Non-goals

- Vim compatibility. A Vim-style preset follows Runyte's selection model and
  is described as Vim-style, never Vim-compatible.
- Shipping a Vim-style preset in 0.5. It lives in `tests/` until a later
  decision.
- Changing `keys.pointer`, the plugin process model, or the MCP tool catalog.
- Accepting the 0.4 shapes of `keys.bind` and `keys.scopes`. See Decisions.

## Decisions

Recorded 2026-11-26.

1. **Conditional steps are included.** Programs may branch on a fixed set of
   conditions. This also prepares user-defined modes (#123).
2. **A program is one undo step.** All text edits made by one binding undo
   together.
3. **`keys.bind` and `keys.scopes` merge into one table, without accepting the
   old forms.** This is a breaking change at the 0.5 minor boundary, as
   `context/reference/releasing.md` permits after an explicit decision.
   Configuration using the old shapes is reported and its bindings dropped,
   with a diagnostic naming the new shape.
4. **The Vim grammar leftovers are removed** before the model changes.
5. **The input layer becomes a small fixed engine, and grammar behaviour moves
   into presets** written in the binding format.
6. **The Vim-style preset is an acceptance test,** not a shipped feature.
7. **Discovery names and keys (2026-11-26).** `describe-key` uses `Space h k`;
   `browse-actions` uses `Space h a`. `Space h` is the Help prefix and
   `Space ?` remains `show-help`. `copy-key-binding` is the companion copy
   command. `keymap-export` has no default key and lands in stage 4 with the
   merged binding table, rather than exporting the superseded 0.4 shapes.

## Model

```
key ──► binding ──► program (steps) ──► actions
                └─► macro (keys) ──► replayed through the keymap
```

### Actions

An action is one semantic operation. Every action has one descriptor, and
dispatch, validation, help, key hints and the action browser all read it.

| Field | Meaning | Replaces |
| --- | --- | --- |
| Name, description, category | As today | `CommandMetadata` |
| Views | The buffer types it works in, with a one-line description of what it does in each, e.g. `terminal: copies the review selection` | Interceptions in `terminal_workflows.rs` and similar |
| Modes | The modes it may be bound in | `scan_editor_modes` |
| Effect | One of: selection, text, mode, view, pane, file, waits for an operand, opens a prompt, runs in the background | `can_continue` |
| Count | Rejects, repeats, or interprets the count | `accepts_count` |
| Argument | None, or its format and whether it replaces the operand or prompt | Palette argument parsing |
| Mutating | Ends at a buffer transaction | `is_mutating` |

Rules:

- **Only mode actions change the mode.** `yank`, `delete-selection`,
  `paste-before`, `toggle-case`, `select-all`, `collapse-selection` and every
  other non-mode action leave the mode alone. Default keys add an explicit
  mode step.
- **An action does the same thing in every view, or its descriptor says how it
  differs.** A difference in what a key does, as opposed to how an operation
  is carried out in a view, belongs in a per-view binding.
- **Whether an action can continue a program follows from its effect.**
  Selection, text, mode, view and pane effects continue. An operand wait
  continues once the operand arrives (see Input layer). A prompt or
  background effect must be the last step unless an argument supplies its
  input.
- **Decision actions** are kept where the decision is made per range, or
  depends on intermediate results that the editor state does not expose
  between steps. `paste-after-or-replace` decides per range: a selection of two
  words and one caret replaces the words and pastes after the caret in one
  transaction. `cut-selection` (today `yank-delete-selection`) records a move
  rather than a copy in a directory buffer. Their descriptors say so.

### Programs

A program is a bounded list of steps run against live editor state. A step is
one of:

- an action, optionally with an argument: `yank`, `{ find-next-char: "(" }`,
  `{ search: TODO }`, `{ pipe: sort }`;
- a conditional: `{ if: <condition>, then: [...], else: [...] }`;
- `{ enter-mode: <mode>, keep-count: true }`, which may carry the pending count
  into the next action in that mode.

Conditions are a fixed, documented set evaluated at that step, for example
`selection-has-text`, `multiple-selections`, `register-empty`, `read-only`,
`{ mode: <mode> }`, `{ view: <view> }`, `recording-macro`, and view-specific ones such
as `tree-entry-expanded`. There are no variables or loops. Nesting depth and
total step count are bounded; stage 5 sets the numbers, starting from today's
16 actions per binding.

Execution rules:

- **Failure.** An error, an unavailable action, or a search or text object that
  finds nothing stops the program. Steps already run stay applied. Every
  action that can report "not found" as information rather than an error must
  mark the program halted, as `search-next` does since #129.
- **Counts.** A count goes to the first step whose descriptor accepts one. A
  program with no such step rejects the count before running. A count carried
  by `keep-count` is multiplied by a count typed in the new mode, as in Vim's
  `2d3w`.
- **Undo.** A program's text edits are one undo step. Selection-only programs
  are one selection-history step, as today. A program that ends in Insert mode
  keeps the undo group open until Insert is left, so `[delete-selection,
  enter-insert-mode]` followed by typing undoes as one change.
- **Programs never type keys and never trigger other bindings.** Their meaning
  does not change when other keys are rebound.

### Bindings

A binding maps (view, mode, keys) to a program or a macro. `global` is a view
like any other, and views inherit from `global` as scoped bindings do today.

```yaml
keys:
  leader: Space
  window: Ctrl-w
  defaults: true
  rebind:
    Space g: Leader G
  bind:
    global:
      normal:
        F6: [select-all, yank]
        Space x: [select-inside-parentheses, delete-selection]
        F8: { macro: next-word }
        F9: null
    terminal:
      normal:
        p: [paste-after, enter-insert-mode]
    directory-tree:
      normal:
        h:
          - if: tree-entry-expanded
            then: [directory-tree-collapse]
            else: [directory-tree-select-parent]
  macros:
    next-word: "m i w * n"
```

- A binding value is an action name, a list of steps, a mapping with a
  `description` and `steps`, `{ macro: <name or keys> }`, or `null`.
- `keys.rebind`, `keys.leader`, `keys.window` and `keys.defaults` keep their
  meaning. `keys.defaults: false` starts `bind` from an empty built-in preset
  layer, keeping the keys it keeps today.
- `keys.ui` and `keys.pointer` are unchanged in 0.5. Moving `keys.ui` input
  owners onto the binding table is an open question.
- Layering: built-in preset, then `keys.rebind`, then `keys.bind`.

### Modes

A mode is a binding table plus behaviour:

| Property | Meaning |
| --- | --- |
| `fallback` | What an unbound key does: `insert-text`, `send-to-terminal`, or `report` |
| `inherits` | A mode whose bindings apply where this one has none |
| `selecting-motions` | Whether motions select or move a caret; today the global `editor.selecting_motions` |
| `after-each` | A program run after every action in this mode |

The built-in modes are Normal, Select, Insert and Replace. Insert and terminal
Insert differ mainly in their fallback. `inherits`, `selecting-motions` per
mode and `after-each` are what user-defined modes (#123) need, and what a
Vim-style operator mode is built from:

```yaml
modes:
  delete-pending:
    inherits: normal
    selecting-motions: true
    after-each: [delete-selection, { enter-mode: normal }]
keys:
  bind:
    global:
      normal:
        d: { enter-mode: delete-pending, keep-count: true }
      delete-pending:
        d: select-line
        i w: select-inside-word
```

A mode is the sanctioned place for state that spans several keys. It is
shown in the status line and cancelled by Escape, so no other cross-key state
is introduced.

### Macros

A macro is a sequence of keys replayed through the live keymap, as recording
already works (`MacroReplay` in `src/app.rs`, with its recursion guard and
work limit).

| | Programs | Macros |
| --- | --- | --- |
| Means | Do these operations | Press these keys |
| Depends on other bindings | No | Yes |
| Validated at load | Names, order, arguments, views, modes | Key spelling |
| Use for | Anything expressible as actions | Interactive gestures across prompts and operands |
| Defined by | Configuration | Configuration or recording |

- `keys.macros.<name>` defines a macro in the key spelling `keys.rebind` uses.
- `{ macro: <name> }` or `{ macro: "<keys>" }` binds one.
- A macro cannot be a step in a program, because a program would have to feed
  input mid-run and then continue. A macro may press keys that run programs.
  A program may end with `replay-macro`.
- Recorded macros stay in memory per editor session. Saving a recording to
  configuration is an open question.
- The macro list in help shows each macro's keys.

### Input layer

`RunyteGrammar` (`src/input_grammar.rs`) reduces to four jobs, and everything
else is preset data:

1. Walk the binding tree for multi-key sequences. Prefixes such as `Space`,
   `g`, `m` and `Ctrl-w` come from bindings, not code.
2. Collect a count and a register prefix (`3`, `"a`) and hand them to the
   program.
3. Capture an operand when an interactive step asks for one, then resume the
   program at the next step. The engine holds one paused program per pane;
   Escape cancels it. This replaces the grammar's current operand wait and is
   the only pause a program can make; prompts and background work stay last.
4. Apply the mode's fallback to keys with no binding.

### Presets

A preset is a complete set of modes and bindings in the binding format. The
built-in keymap becomes the Runyte preset, authored in that format and
compiled into static tables at build time so startup does not parse YAML
(see `context/reference/startup-performance.md`). A preset that fails
validation fails the build.

The Vim-style preset lives in `tests/` and covers a defined core: the operators
`d`, `c`, `y`, `>`, `<`; motions and text objects after an operator; doubled
operators (`dd`, `yy`, `cc`); counts before the operator and before the motion;
`.`; and Visual and Visual-line modes. Its tests drive it through
`src/headless.rs`. Anything in that core that cannot be expressed is a gap in
the model, not in the preset. Known differences that remain: Runyte's cursor
always covers one character, Vim-specific edge cases of motion rules, and Vim
features outside bindings (marks, ex ranges, `:normal`).

### Discoverability

- **Describe key.** After a key press, shows the view, mode, source (built-in
  preset, or configuration file and line) and each step with its description,
  with a command to copy the binding as YAML.
- **Action browser.** A searchable list on the shared picker
  (`src/picker.rs`). Each row shows the name, description, per-view
  behaviour, effect, count and argument, and default keys. It can be filtered
  to actions usable in the current view and mode.
- **Generated help.** `:help key-actions`, key hints and the action reference
  in `docs/user-guide/custom-key-bindings.md` are generated from descriptors.
- **Export.** `:keymap-export` writes the effective keymap as YAML.
- **Key hints** show a binding's description, or its step names when it has
  none.

Discovery names and default keys follow decision 7 and are recorded in
`context/reference/helix-keymap-v1.md`. Whole-keymap export lands in stage 4.

## Initial classification of existing actions

Stage 3 completes this against all actions. A test then requires every action
to be atomic or a documented decision action.

| Today | Becomes | Default key binds |
| --- | --- | --- |
| `yank-collapse-normal-mode`, `yank-line-collapse-normal-mode` | Removed | `y`: `[yank, collapse-selection, enter-normal-mode]`; `collapse-selection` takes over the half-open handling in `collapse_after_yank` |
| `paste-after-or-replace` | Decision action, without its mode change | `p`: `[paste-after-or-replace, enter-normal-mode]` |
| `paste-before` | Leaves the mode alone | `P`: `[paste-before, enter-normal-mode]` |
| `toggle-case`, `replace-char` | Leave the mode alone | `~`, `r`: the action, then `enter-normal-mode`; `r` resumes after its operand |
| `delete-selection`, `change-selection` | `delete-selection` leaves the mode alone; `change-selection` removed | `Alt-d`, `Alt-c` when bound: `[delete-selection, enter-normal-mode]` or `[delete-selection, enter-insert-mode]` |
| `yank-delete-selection`, `yank-change-selection` | One decision action, `cut-selection` | `d`: `[cut-selection, enter-normal-mode]`; `c`: `[cut-selection, enter-insert-mode]` |
| `search-selection` | Removed | `*`: `[set-search-from-selection, select-all-search-matches, enter-select-mode]` |
| `search`, `search-regex` | Prompt actions; with an argument they run without a prompt | `s`, `/`: unchanged behaviour as programs ending in Select mode |
| `search-next`, `search-previous` | Leave the mode alone; new `extend-search-next`, `extend-search-previous` | `n`, `N`: the action, then `enter-select-mode` |
| `select-all`, `collapse-selection` | Leave the mode alone | `%`: `[select-all, enter-select-mode]`; `;`: `[collapse-selection, enter-normal-mode]` |
| `enter-select-mode` (toggles) | `enter-select-mode`, `exit-select-mode`; toggle as a program | `v`: `{ if: { mode: select }, then: [collapse-selection, enter-normal-mode], else: [enter-select-mode] }` |
| `clipboard-paste-after`, `clipboard-paste-before` | Mirror the register actions | `Space c p`, `Space c P`: as `p`, `P` |
| `directory-tree-left`, `directory-tree-right`, `directory-tree-open` | Split into collapse, select-parent, expand, select-first-child, open | Conditional programs in `directory-tree` |
| `media-back`, `media-move-up`, `media-move-down` | Split into their steps | Conditional programs in `media` |
| `record-default-macro` (toggles) | Start only | `{ if: recording-macro, then: [stop-macro-recording], else: [record-default-macro] }` |
| Terminal interceptions of `yank`, paste and search | Terminal implementations in the descriptors; mode differences as `terminal` bindings | `terminal` `p`: `[paste-after, enter-insert-mode]` |
| `join-lines`, insert-entry actions (`append-after`, `open-line-below`, ...) | Unchanged | Unchanged |

Bundled action names that are removed are reported when configuration names
them, with the program that replaces them.

## Delivery stages

Each stage keeps `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test` and the coverage floor in
`context/reference/test-coverage.md` passing, plus the desktop-edition checks
where input or help can reach the native frontend.

0. **Remove the Vim grammar leftovers.** Delete `VimGrammar`, the
   `RangeIntent::Vim*` variants and handlers, `find_search`,
   `search_selection_direction`, `SearchForward`, `SearchBackward`, the
   Vim-only selection semantics and `GrammarKind::Vim`. Keep half-open
   semantics, which syntax selection and the mouse use. Re-measure coverage.
1. **Action descriptors.** One descriptor per action. Derive modes,
   continuation and counts from it, with no behaviour change. A test requires
   a complete descriptor for every action. During this migration the descriptor
   retains `legacy_sequence_position` beside descriptive effects, preserving
   existing sequence admission. The writable-buffer gate remains separate from
   possible text transactions; scoped admission remains authoritative until
   stage 4.
2. **Discoverability on the current keymap.** Describe key, the action
   browser, and help generated from descriptors.
3. **Programs everywhere.** Built-in bindings become programs. Add the count
   and undo rules, operand resumption, and the atomic splits in the
   classification above. Author the built-in keymap as the Runyte preset,
   compiled at build time. Remove `legacy_sequence_position` when execution and
   validation switch together to effect-derived continuation. Default keys keep
   their behaviour; the classification test lands.
4. **One binding table.** Merge `keys.bind` and `keys.scopes` into
   `keys.bind.<view>.<mode>`. Move terminal and directory interceptions into
   descriptors and `terminal` and `directory` bindings. Add per-mode fallback.
   Report the old configuration shapes. Add `:keymap-export` (no default key)
   to export this merged table as YAML.
5. **Arguments and conditionals.** Arguments that replace operands and
   prompts, `insert-text`, and conditional steps with the documented
   condition set.
6. **Macros in configuration.** `keys.macros`, `{ macro: ... }` bindings, and
   the macro list showing keys.
7. **Mode hooks and the Vim-style acceptance preset.** `inherits`, per-mode
   `selecting-motions`, `after-each`, `keep-count` and `repeat-last-change`.
   Exit criterion: the Vim-style preset core passes in `tests/`. User-defined
   modes themselves are delivered with #123.
8. **Release 0.5.0.** Migration notes for the configuration change, updated
   user guide, and an updated `context/reference/helix-keymap-v1.md`.

## Verification

- Descriptor completeness and classification tests over every action.
- Every built-in binding validated at build time and listed by `:help
  key-actions`.
- Program semantics: counts, undo grouping including programs ending in
  Insert, failure stops, conditional branches, operand resumption and Escape
  cancellation.
- Per-view behaviour: terminal and directory bindings and descriptors.
- Macros: configured definitions, binding, recursion and work limits.
- The Vim-style acceptance preset through the headless facade.
- Startup measured with `benchmarks/` and recorded in
  `context/reference/startup-performance.md` after stage 3.
- Integration tests isolate configuration, cache, state and account paths as
  AGENTS.md requires.

## Open questions

1. Should `keys.ui` input owners move onto the binding table in 0.5, while the
   configuration is breaking anyway?
2. Should the built-in preset be YAML compiled by `build.rs`, or Rust tables
   using the same types with a test that renders them as YAML?
3. Should `:macro-save` write a recorded macro into `keybinds.d/`? Today the
   settings page never writes `keys`.
4. Should macros support literal text segments, or should text go through
   programs with `insert-text`?
5. Should plugin commands be allowed as the last step of a program?
6. Which conditions belong in the first condition set, and which are
   view-specific?

## Documents to update

- `docs/user-guide/custom-key-bindings.md` and `docs/user-guide/key-bindings.md`
- `context/reference/helix-keymap-v1.md`
- `context/reference/ui-vocabulary.md` where lists and pickers change
- `context/reference/startup-performance.md`
- `context/plans/README.md`

### Stage 3 implementation slices

Stage 3a introduces a nonempty bounded program for every binding, retaining
inline storage for singleton defaults and explicit `keys.bind` provenance for
input ownership. Multi-step programs route a count to their first accepting
step, or reject it before any effects when none accepts it. Singleton key
behavior remains in a named compatibility adapter: for example, `2fx` still
ignores the count, while a parameterized singleton rejects it. The final
uniform count policy needs to reconcile this with default-key compatibility.

Stage 3b still owns resumable execution, program undo ownership, effect-derived
continuation and removal of the legacy policy, atomic splits and default
program rewrites. Until those land, operands and mode changes retain their
existing last-step restrictions and text edits retain their existing undo
boundaries. Preset authoring remains pending open question 2. Stage 3 is not
complete with 3a alone.
