<!-- SPDX-License-Identifier: MPL-2.0 -->
# Composable key bindings: actions, programs, macros and presets

Tracking issue: [#132](https://github.com/runyte/runyte/issues/132).
Status: active, design approved 2026-11-26 and revised 2026-11-27 after review
feedback on #132. Breaking change, released as 0.5.0.
Related: [#123](https://github.com/runyte/runyte/issues/123) (modes per
buffer type, delivered by stage 7), [#124](https://github.com/runyte/runyte/issues/124) and
[#127](https://github.com/runyte/runyte/issues/127) (atomic splits),
[PR #129](https://github.com/runyte/runyte/pull/129).

## Terms

These follow the glossary in `AGENTS.md`.

- A **buffer type** is the kind of content a pane shows: text, terminal,
  directory, directory tree, media, a Git list, settings and so on. Earlier
  revisions of this plan and the code call it a *view* or *scope*
  (`BindingScope`); user-facing text says buffer type.
- A **mode** is a persistent input state with its own bindings, such as
  Normal, Select, Insert or Replace. Every buffer type starts in a mode.
- A **submode** is a short-lived mode entered for a few keys and left
  automatically, such as an operator-pending `delete-pending`. Submodes use
  the same machinery as modes.
- An **action** is one semantic operation with a descriptor. A **program** is
  a bounded list of steps (actions, conditionals, mode changes). A **macro** is
  a sequence of keys replayed through the keymap. A **preset** is a complete
  set of modes and bindings.

## Problem

Custom bindings are hard to build because the parts they are built from are
inconsistent. #124 and #129 split `yank`, `paste-after` and `*` into atomic
actions and kept the old behaviour under new names. Repeating that for each
remaining command would double the action list without making it easier to
understand. The causes are structural:

1. **Six configuration surfaces with different rules.** `keys.bind` works per
   mode and accepts lists and arguments. `keys.scopes` works per buffer type
   but takes one action, without arguments or lists. `keys.ui` has its own
   action names per input owner. `keys.pointer`, `keys.rebind` and
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
4. **Per-buffer-type behaviour is hidden.** A terminal intercepts `yank`,
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
   was compiled only for tests, and its handlers remained compiled but
   unreachable. Stage 0 removed them.
8. **Modes are hard-coded.** Normal, Select, Insert and Replace are an enum
   checked in about 700 places outside tests. A mode cannot be added, removed
   or given different behaviour per buffer type, which #123 needs.

There are 367 editor actions. Part of that count is bundled duplicates; part
is actions whose behaviour depends on the buffer type without saying so.

## Goals

- A user can see exactly what any key does: the actions it runs, in order, and
  where the binding comes from.
- A user can list and search every action, with what it does in each buffer
  type and mode, whether it can be followed by another action, and its count
  and argument.
- One binding format for every buffer type and mode, used by the built-in
  preset and by user configuration alike. The built-in preset is minimally
  special: it is written in the user format and is one of possibly several
  shipped presets, like themes.
- Atomic actions only, except where an action makes a decision that a program
  cannot express. Each such action is documented as a decision.
- A clear divide between actions (semantic operations) and macros (replayed
  key input), with macros definable and bindable in configuration.
- Modes defined by presets and configuration rather than code, so that
  user-defined modes (#123) and different editing styles are configuration.
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
- Removing any current mode from the default experience. The Runyte preset
  keeps Normal, Select, Insert and Replace.

## Decisions

Initial decisions recorded 2026-11-26; later decisions are dated. Decisions
15 to 27 follow the review feedback on #132 (comments 7204869913 and
7205719368) and were made by the maintainer on 2026-11-27. Where a later
decision revises an earlier one, the earlier entry says so.

1. **Conditional steps are included.** Programs may branch on a fixed set of
   conditions. This also prepares user-defined modes (#123).
2. **A program is one undo step.** All text edits made by one binding undo
   together.
3. **`keys.bind` and `keys.scopes` merge into one table, without accepting the
   old forms.** This is a breaking change at the 0.5 minor boundary, as
   `context/reference/releasing.md` permits after an explicit decision.
   Configuration using the old shapes is reported and its bindings dropped,
   with a diagnostic naming the new shape. Decision 19 sets the table's shape.
4. **The Vim grammar leftovers are removed** before the model changes.
5. **The input layer becomes a small fixed engine, and grammar behaviour moves
   into presets** written in the binding format.
6. **The Vim-style preset is an acceptance test,** not a shipped feature.
7. **Discovery names and keys (2026-11-26).** `describe-key` uses `Space h k`;
   `browse-actions` uses `Space h a`. `Space h` is the Help prefix and
   `Space ?` remains `show-help`. `copy-key-binding` is the companion copy
   command. `keymap-export` has no default key and lands in stage 4 with the
   merged binding table, rather than exporting the superseded 0.4 shapes.
8. **Rust-authored preset (decided 2026-11-26; superseded by decision 25).**
   The preset was to be Rust tables rendered to checked-in YAML by a test.
9. **Uniform counts (decided 2026-11-27).** Every binding sends its count to
   the first step whose descriptor accepts one. If none accepts it, the count
   is dropped silently and the binding runs. This applies to built-in and
   configured bindings, single-step and multi-step programs, and editor,
   colon, plugin and parameterized actions. `2fx` remains equivalent to `fx`;
   a count before `[select-all, yank]` still runs both steps. Decision 18 adds
   per-step overrides; this remains the default.
10. **UI and pointer boundaries stay separate (decided 2026-11-27, delegated
    by the maintainer).** `keys.ui` and `keys.pointer` remain separate in 0.5.
    UI input owners are input boundaries, not programs. Revisit after 0.5.
11. **Copy recorded macros (decided 2026-11-27, delegated by the maintainer;
    revised by decision 26).** Stage 6 adds `copy-macro`, which copies YAML
    for `keys.macros`, following the `copy-key-binding` model.
12. **Macros contain keys only (decided 2026-11-27, delegated by the
    maintainer; refined by decision 27).** Macros have no text segments that
    bypass the keymap. Typed text is keys like any other.
13. **Plugin commands end programs (decided 2026-11-27, delegated by the
    maintainer).** A plugin command is allowed only as the last step because
    it runs out of process; nothing may follow it.
14. **Bounded condition inventory (decided 2026-11-27, delegated by the
    maintainer).** The initial general conditions are `selection-has-text`,
    `multiple-selections`, `register-empty`, `read-only`, `{ mode: <mode> }`,
    `{ buffer-type: <type> }` and `recording-macro`. Add exactly the
    buffer-type-specific conditions required by the default directory-tree and
    media splits, including `tree-entry-expanded`, `tree-entry-is-directory`
    and those needed by the media splits. Each condition documents its
    applicable buffer types; one used outside them is rejected at load. Add a
    condition only for a default binding or a documented use. Internal
    conditional steps may serve stage 3 defaults; public syntax lands in
    stage 5.
15. **Terminology.** User-facing text and this plan say *buffer type* rather
    than view or scope, keep *mode* for persistent input states, and say
    *submode* for short-lived ones. The terms are added to the glossary in
    `AGENTS.md`. Internal Rust names may be renamed when the code they belong
    to is rewritten; renaming alone is not a reason to churn code.
16. **One meaning per action.** An action keeps one meaning in every buffer
    type: there is no `yank-text` beside `yank-terminal`. A descriptor may say
    how a buffer type carries an operation out (a terminal copies its review
    selection). Where the *meaning* differs, the buffer type gets its own
    action, bound in that buffer type's mode, instead of one action branching
    internally. Stage 4 applies this rule to today's interceptions.
17. **Step syntax.** A step is an action name, or a mapping
    `{ action: <name>, arg: <value> }` with optional `count`, `register` and
    `extend` fields. This replaces the `{ <name>: <arg> }` form, which is not
    accepted. Conditionals keep `{ if: ..., then: ..., else: ... }`.
18. **Per-step count and register.** A step may set `count: inherit` (this
    step also receives the binding's count), `count: none` (it never does) or
    `count: <n>` (a fixed count). Without a field, decision 9 applies. A
    register prefix (`"a`) follows the same rule: it goes to the first step
    that uses a register, and `register: inherit | none | <name>` overrides
    that per step.
19. **Modes are the only table dimension.** The table is
    `keys.bind.<mode>.<keys>`. Each buffer type declares the mode it starts
    in under `buffer-types:`; nothing is inferred from names. Keys shared by
    several modes live in a mode that the others inherit (the Runyte preset
    calls it `common`), not in a separate global section and not by copying.
    Lookup has one chain: the mode's own bindings, then its `inherits` chain,
    then the mode's `unbound-keys` behaviour.
20. **Editing modes come from presets.** The engine provides mode properties
    (decision 23) and keeps only the UI input states (the command prompt and
    lists) as code. Every editing mode, including Normal, Select, Insert and
    Replace, is defined by the bundled Runyte preset, which keeps all four by
    default. Select is `{ inherits: normal, selecting-motions: true }`;
    Replace is `{ unbound-keys: overwrite }`. The engine requires every buffer
    type to have a starting mode. A preset may omit Select and extend
    selections with `{ action: <motion>, extend: true }` steps instead, as a
    Kakoune-style preset would.
21. **Action names: subject first for families.** An action that belongs to
    one subject other than the text or selection itself is named subject
    first, so a sorted or prefix-filtered list groups it: `search-`, `mode-`,
    `register-`, `syntax-`, `macro-`, `clipboard-`, and the existing families
    (`git-`, `session-`, `directory-`, `directory-tree-`, `media-`,
    `terminal-`). Core editing and motion verbs that act on the text or the
    selection keep their names: `yank`, `paste-*`, `delete-*`, `undo`,
    `redo`, `move-*`, `goto-*`, `select-*` text objects. Examples:
    `set-search-from-selection` becomes `search-set-from-selection`, and
    entering a mode becomes `{ action: mode-enter, arg: select }`. New names
    follow the rule from stage 3b; existing family members are renamed in
    stage 4, and configuration naming an old name is reported with its new
    one.
22. **Implicit mode declaration.** A mode exists once a preset or
    configuration binds keys in it or enters it. `modes:` is needed only to
    set properties. A mode that is entered but has no bindings and no
    declaration is reported at load, which catches misspelt names.
23. **Mode properties are one documented schema.** `fallback` is renamed
    `unbound-keys`, with the values `pass-to-buffer` (a text buffer inserts
    the text, a terminal sends the key to its child), `overwrite` and
    `report`. Each property has a documented set of values and a default.
    The schema lives in code and renders into the user guide and `:help
    modes`. `describe-mode` on `Space h m` shows the current mode's effective
    properties, its `inherits` chain and where each value comes from. Describe
    key names the mode in the chain that supplied a binding.
24. **Prefixes stay key sequences.** `Space x` remains a two-key sequence, not
    a declared `space-menu` submode. A pending prefix behaves like a one-shot
    submode (key hints, Escape cancels), and the documentation says so.
25. **YAML presets (supersedes decision 8).** Presets are written in the user
    binding format and embedded in the binary, like themes. The Runyte preset
    is the default and others may ship later; `keys.preset` selects one.
    There is no `build.rs` compilation and no Rust preset table. A test
    validates every embedded preset and requires that the Runyte preset
    produces today's default keys. Parsing the preset at startup is measured
    with `benchmarks/`; if it exceeds the startup budget in
    `context/reference/startup-performance.md`, the compiled preset is cached,
    and the cache is invalidated by the build.
26. **Saved macros live in `macros.d/` (revises decision 11).** Configured
    macros may be defined in `keys.macros` or in YAML files under `macros.d/`
    beside `keybinds.d/`. `:macro-save <name>` writes a recorded macro to
    `macros.d/<name>.yaml`, as `:write-config` already writes `keybinds.d/`.
    `copy-macro` remains for copying one macro as YAML.
27. **Compact macro notation (refines decision 12).** A macro is spelled with
    plain characters as keys and named keys in angle brackets, so
    `ihello<Esc>` types `hello` in Insert mode and returns to Normal.
    `<Space>`, `<lt>` and modifier forms such as `<C-w>` cover keys that are
    not plain characters.

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
| Name, description, category | As today; names follow decision 21 | `CommandMetadata` |
| Buffer types | The buffer types it works in, with a one-line description of how each carries it out, e.g. `terminal: copies the review selection` | Interceptions in `terminal_workflows.rs` and similar |
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
- **One meaning per action** (decision 16). How an operation is carried out
  may differ per buffer type and is described in the descriptor; a different
  meaning is a different action, bound in that buffer type's mode.
- **Whether an action can continue a program follows from its effect.**
  Selection, text, mode, view and pane effects continue. An operand wait
  continues once the operand arrives (see Input layer). A prompt or
  background effect must be the last step unless an argument supplies its
  input. Plugin commands are permitted only as the last step because they run
  out of process (decision 13).
- **Decision actions** are kept where the decision is made per range, or
  depends on intermediate results that the editor state does not expose
  between steps. `paste-after-or-replace` decides per range: a selection of two
  words and one caret replaces the words and pastes after the caret in one
  transaction. `cut-selection` (today `yank-delete-selection`) records a move
  rather than a copy in a directory buffer. Their descriptors say so.

### Programs

A program is a bounded list of steps run against live editor state. A step is
one of:

- an action name: `yank`;
- an action with fields (decision 17): `{ action: find-next-char, arg: "(" }`,
  `{ action: search, arg: TODO }`, `{ action: pipe, arg: sort }`,
  `{ action: paste-after, count: none }`, `{ action: move-word-forward,
  extend: true }`;
- a conditional: `{ if: <condition>, then: [...], else: [...] }`;
- a mode change: `{ action: mode-enter, arg: <mode>, keep-count: true }`,
  which may carry the pending count into the next action in that mode.

Conditions are a fixed, documented set evaluated at that step (decision 14).
There are no variables or loops. Nesting depth and total step count are
bounded; stage 5 sets the numbers, starting from today's 16 actions per
binding.

Execution rules:

- **Failure.** An error, an unavailable action, or a search or text object that
  finds nothing stops the program. Steps already run stay applied. Every
  action that can report "not found" as information rather than an error must
  mark the program halted, as `search-next` does since #129.
- **Counts.** A count goes to the first step whose descriptor accepts one. A
  program with no such step drops the count silently and runs (decision 9).
  A step's `count` field overrides this for that step (decision 18). A count
  carried by `keep-count` is multiplied by a count typed in the new mode, as
  in Vim's `2d3w`.
- **Registers.** A register prefix goes to the first step that uses a
  register, unless a step's `register` field says otherwise (decision 18).
- **Undo.** A program's text edits are one undo step. Selection-only programs
  are one selection-history step, as today. A program that ends in Insert mode
  keeps the undo group open until Insert is left, so `[delete-selection,
  { action: mode-enter, arg: insert }]` followed by typing undoes as one
  change.
- **Programs never type keys and never trigger other bindings.** Their meaning
  does not change when other keys are rebound.

### Bindings

A binding maps (mode, keys) to a program or a macro (decision 19). Buffer
types choose their starting mode; shared keys come from an inherited mode.

```yaml
buffer-types:
  text: normal
  terminal: terminal
  directory-tree: directory-tree
modes:
  normal:          { inherits: common }
  select:          { inherits: normal, selecting-motions: true }
  insert:          { unbound-keys: pass-to-buffer }
  terminal:        { inherits: common }
  directory-tree:  { inherits: common }
keys:
  preset: runyte
  leader: Space
  window: Ctrl-w
  defaults: true
  rebind:
    Space g: Leader G
  bind:
    common:
      Space f: open-file-picker
    normal:
      F6: [select-all, yank]
      Space x: [select-inside-parentheses, delete-selection]
      F8: { macro: next-word }
      F9: null
    terminal:
      p: [paste-after, { action: mode-enter, arg: insert }]
    directory-tree:
      h:
        - if: tree-entry-expanded
          then: [directory-tree-collapse]
          else: [directory-tree-select-parent]
  macros:
    next-word: "miw*n"
```

The `buffer-types` and `modes` sections above are what the Runyte preset
already defines; a user writes them only to change them.

- A binding value is an action name, a list of steps, a mapping with a
  `description` and `steps`, `{ macro: <name or keys> }`, or `null`.
- `keys.rebind`, `keys.leader`, `keys.window` and `keys.defaults` keep their
  meaning. `keys.defaults: false` starts `bind` from an empty preset layer,
  keeping the keys it keeps today.
- `keys.ui` and `keys.pointer` are unchanged in 0.5 (decision 10).
- Layering: the selected preset, then `keys.rebind`, then `keys.bind`.

### Modes

A mode is a binding table plus properties (decision 23):

| Property | Values | Default | Meaning |
| --- | --- | --- | --- |
| `inherits` | A mode name, or none | none | A mode whose bindings apply where this one has none |
| `unbound-keys` | `pass-to-buffer`, `overwrite`, `report` | `report` | What a key with no binding in the chain does |
| `selecting-motions` | `true`, `false` | `false`, or the inherited value | Whether motions extend the selection or move a caret; replaces the global `editor.selecting_motions` |
| `after-each` | A program, or none | none | A program run after every action in this mode |
| `display-name` | Text | The mode name | What the status line shows |

The exact set and defaults are fixed in stage 4 (`inherits`, `unbound-keys`)
and stage 7 (the rest), and rendered from the schema into the guide and
`:help modes`.

The Runyte preset defines Normal, Select, Insert and Replace (decision 20).
User-defined modes and submodes use the same properties, which is what #123
needs and what a Vim-style operator submode is built from:

```yaml
modes:
  delete-pending:
    inherits: normal
    selecting-motions: true
    after-each: [delete-selection, { action: mode-enter, arg: normal }]
keys:
  bind:
    normal:
      d: { action: mode-enter, arg: delete-pending, keep-count: true }
    delete-pending:
      d: select-line
      i w: select-inside-word
```

A mode is the sanctioned place for state that spans several keys. It is
shown in the status line and cancelled by Escape, so no other cross-key state
is introduced. A pending key prefix behaves like a one-shot submode
(decision 24).

### Macros

A macro is a sequence of keys replayed through the live keymap, as recording
already works (`MacroReplay` in `src/app.rs`, with its recursion guard and
work limit).

| | Programs | Macros |
| --- | --- | --- |
| Means | Do these operations | Press these keys |
| Depends on other bindings | No | Yes |
| Validated at load | Names, order, arguments, buffer types, modes | Key spelling |
| Use for | Anything expressible as actions | Interactive gestures across prompts and operands, typed text |
| Defined by | Configuration | Configuration, `macros.d/`, or recording |

- `keys.macros.<name>` or a file under `macros.d/` defines a macro in the
  compact notation (decision 27): `"ihello<Esc>"`, `"miw*n"`.
- `{ macro: <name> }` or `{ macro: "<keys>" }` binds one.
- A macro cannot be a step in a program, because a program would have to feed
  input mid-run and then continue. A macro may press keys that run programs.
  A program may end with `macro-replay`.
- Recorded macros stay in memory per editor session until saved.
  `:macro-save <name>` writes one to `macros.d/<name>.yaml`; `copy-macro`
  copies one as YAML (decision 26).
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
4. Apply the current mode's `unbound-keys` behaviour to keys with no binding in
   its chain.

### Presets

A preset is a complete set of buffer-type starting modes, modes and bindings
in the user binding format (decision 25). Presets are embedded in the binary,
like themes, and `keys.preset` selects one; the Runyte preset is the default.
A test validates every embedded preset and requires that the Runyte preset
reproduces today's default keys. Startup cost of parsing the preset is
measured and recorded in `context/reference/startup-performance.md`.

The Vim-style preset lives in `tests/` and covers a defined core: the operators
`d`, `c`, `y`, `>`, `<`; motions and text objects after an operator; doubled
operators (`dd`, `yy`, `cc`); counts before the operator and before the motion;
`.`; and Visual and Visual-line modes. Its tests drive it through
`src/headless.rs`. Anything in that core that cannot be expressed is a gap in
the model, not in the preset. Known differences that remain: Runyte's cursor
always covers one character, Vim-specific edge cases of motion rules, and Vim
features outside bindings (marks, ex ranges, `:normal`).

### Discoverability

- **Describe key.** After a key press, shows the buffer type, mode, the mode
  in the inheritance chain that supplied the binding, its source (preset, or
  configuration file and line) and each step with its description, with a
  command to copy the binding as YAML.
- **Describe mode.** `describe-mode` on `Space h m` shows the current mode's
  effective properties, its `inherits` chain and where each value comes from
  (decision 23).
- **Action browser.** A searchable list on the shared picker
  (`src/picker.rs`). Each row shows the name, description, per-buffer-type
  behaviour, effect, count and argument, and default keys. It can be filtered
  to actions usable in the current buffer type and mode.
- **Generated help.** `:help key-actions`, `:help modes`, key hints and the
  action reference in `docs/user-guide/custom-key-bindings.md` are generated
  from descriptors and the mode schema.
- **Export.** `:keymap-export` writes the effective keymap as YAML.
- **Key hints** show a binding's description, or its step names when it has
  none.

Discovery names and default keys follow decisions 7 and 23 and are recorded
in `context/reference/helix-keymap-v1.md`.

## Initial classification of existing actions

Stage 3 completes this against all actions. A test then requires every action
to be atomic or a documented decision action. New and renamed actions follow
decision 21; the names below are working names until stage 3b settles them.
Mode steps are written as `enter-<mode>-mode` for brevity; in the new step
syntax they are `{ action: mode-enter, arg: <mode> }`.

| Today | Becomes | Default key binds |
| --- | --- | --- |
| `yank-collapse-normal-mode`, `yank-line-collapse-normal-mode` | Removed | `y`: `[yank, collapse-selection, enter-normal-mode]`; `collapse-selection` takes over the half-open handling in `collapse_after_yank` |
| `paste-after-or-replace` | Decision action, without its mode change | `p`: `[paste-after-or-replace, enter-normal-mode]` |
| `paste-before` | Leaves the mode alone | `P`: `[paste-before, enter-normal-mode]` |
| `toggle-case`, `replace-char` | Leave the mode alone | `~`, `r`: the action, then `enter-normal-mode`; `r` resumes after its operand |
| `delete-selection`, `change-selection` | `delete-selection` leaves the mode alone; `change-selection` removed | `Alt-d`, `Alt-c` when bound: `[delete-selection, enter-normal-mode]` or `[delete-selection, enter-insert-mode]` |
| `yank-delete-selection`, `yank-change-selection` | One decision action, `cut-selection` | `d`: `[cut-selection, enter-normal-mode]`; `c`: `[cut-selection, enter-insert-mode]` |
| `search-selection` | Removed | `*`: `[search-set-from-selection, search-select-all-matches, enter-select-mode]` |
| `search`, `search-regex` | Prompt actions; with an argument they run without a prompt | `s`, `/`: unchanged behaviour as programs ending in Select mode |
| `search-next`, `search-previous` | Leave the mode alone; new `search-extend-next`, `search-extend-previous` | `n`, `N`: the action, then `enter-select-mode` |
| `select-all`, `collapse-selection` | Leave the mode alone | `%`: `[select-all, enter-select-mode]`; `;`: `[collapse-selection, enter-normal-mode]` |
| `enter-select-mode` (toggles) | `mode-enter` with an argument; toggle as a program | `v`: `{ if: { mode: select }, then: [collapse-selection, enter-normal-mode], else: [enter-select-mode] }` |
| `clipboard-paste-after`, `clipboard-paste-before` | Mirror the register actions | `Space c p`, `Space c P`: as `p`, `P` |
| `directory-tree-left`, `directory-tree-right`, `directory-tree-open` | Split into collapse, select-parent, expand, select-first-child, open | Conditional programs in `directory-tree` |
| `media-back`, `media-move-up`, `media-move-down` | Split into their steps | Conditional programs in `media` |
| `record-default-macro` (toggles) | Start only | `{ if: recording-macro, then: [macro-stop-recording], else: [macro-record-default] }` |
| Terminal interceptions of `yank`, paste and search | Same meaning: described in the descriptor. Different meaning: terminal actions bound in the `terminal` mode (decision 16) | `terminal` `p`: `[paste-after, enter-insert-mode]` |
| `join-lines`, insert-entry actions (`append-after`, `open-line-below`, ...) | Unchanged | Unchanged |

Bundled action names that are removed are reported when configuration names
them, with the program that replaces them.

## Delivery stages

Each stage keeps `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test` and the coverage floor in
`context/reference/test-coverage.md` passing, plus the desktop-edition checks
where input or help can reach the native frontend.

0. **Remove the Vim grammar leftovers.** Done in #134.
1. **Action descriptors.** Done in #137. During the migration the descriptor
   retains `legacy_sequence_position` beside descriptive effects, preserving
   existing sequence admission. The writable-buffer gate remains separate from
   possible text transactions; scoped admission remains authoritative until
   stage 4.
2. **Discoverability on the current keymap.** Done in #138.
3. **Programs everywhere.** Built-in bindings become programs. Add the count
   and undo rules, operand resumption, and the atomic splits in the
   classification above, with new names following decision 21. Remove
   `legacy_sequence_position` when execution and validation switch together
   to effect-derived continuation. Author the built-in keymap as the embedded
   YAML Runyte preset (decision 25), in the 0.4 binding shapes until stage 4,
   and measure its startup cost. Default keys keep their behaviour; the
   classification test lands.
4. **One binding table, modes as its only dimension.** Replace `keys.bind`
   and `keys.scopes` with `keys.bind.<mode>` (decision 19). Add
   `buffer-types:`, implicit mode declaration (decision 22), `inherits` and
   `unbound-keys`, with the preset's `common` mode replacing global bindings.
   Apply decision 16 to the terminal and directory interceptions. Rename
   existing family actions (decision 21), reporting old names. Report the old
   configuration shapes. Add `:keymap-export` (no default key) and migrate the
   Runyte preset to the new shape.
5. **Step syntax, arguments and conditionals.** The `{ action:, arg: }` step
   form with `count`, `register` and `extend` fields (decisions 17 and 18),
   arguments that replace operands and prompts, `insert-text`, and
   conditional steps with the documented condition set.
6. **Macros in configuration.** `keys.macros`, `macros.d/`, the compact
   notation, `{ macro: ... }` bindings, `:macro-save`, `copy-macro`, and the
   macro list showing keys (decisions 26 and 27).
7. **Modes from presets, and the Vim-style acceptance preset.** Define Normal,
   Select, Insert and Replace in the Runyte preset and replace the hard-coded
   mode checks with mode properties (decision 20). Add per-mode
   `selecting-motions`, `after-each`, `display-name`, `keep-count`,
   `macro-replay` and `repeat-last-change`. Render the mode schema into the
   guide and `:help modes`, and add `describe-mode` on `Space h m`
   (decision 23). User-defined modes and submodes become configuration,
   resolving #123. Exit criterion: the Vim-style preset core passes in
   `tests/`, and a test preset without Select extends selections with
   `extend: true` steps.
8. **Release 0.5.0.** Migration notes for the configuration change, updated
   user guide, and an updated `context/reference/helix-keymap-v1.md`.

### Stage 3 implementation slices

Stage 3a (#139) introduced a nonempty bounded program for every binding,
retaining inline storage for singleton defaults and explicit `keys.bind`
provenance for input ownership. Every program routes a count to its first
accepting step, or drops it silently when none accepts it, following
decision 9.

The first stage 3b slice replaces eager program translation with an acknowledged
cursor: the application executes a step before preparing the next one. Operand
waits retain the cursor, count recipient, feedback and selection-history origin.
A paused program is cancelled with a status message when pointer or command
focus leaves its pane; its document or terminal review,
selection, mode and binding scope must still match when its operand arrives.
Escape cancels the remaining steps, and replay reservations belong to a specific
replay run. Internal constructed programs test operands in the middle of a
program; public admission retains the existing last-step restrictions until the
continuation-policy slice.

The next stage 3b slice gives each binding undo ownership in every buffer it
edits. Ownership survives operand waits and deferred counted replay commands;
completion commits it or hands the active document's group to Insert/Replace.
Cancellation, failure and unrelated edits close the owned checkpoints. Explicit
undo, redo and checkpoint actions remain history barriers. Selection-only
programs retain one selection-history step, including cancelled counted replay.

The continuation slice derives admission and execution from descriptor effects,
removing the legacy sequence policy and its frozen migration fixture. Operand
waits and synchronous effects can continue; remaining prompt/background effects
and plugins are last-only. Supplying prompt input removes only that prompt,
not independent background work. Queued macro replay describes its background
effect explicitly. Mixed-effect bundled actions remain last-only until narrowed.

Stage 3b still owns atomic splits and default program rewrites, and the embedded
YAML Runyte preset (decision 25).

## Verification

- Descriptor completeness and classification tests over every action.
- Every embedded preset validated by tests; the Runyte preset reproduces
  today's default keys, and every binding is listed by `:help key-actions`.
- Program semantics: counts and registers with and without step overrides,
  undo grouping including programs ending in Insert, failure stops,
  conditional branches, operand resumption and Escape cancellation.
- Mode semantics: the lookup chain, implicit declaration and its typo report,
  each `unbound-keys` value, and the mode schema matching its rendered help.
- Per-buffer-type behaviour: terminal and directory modes and descriptors.
- Macros: configured and `macros.d/` definitions, compact notation, binding,
  `:macro-save`, recursion and work limits.
- The Vim-style acceptance preset and a preset without Select, through the
  headless facade.
- Startup measured with `benchmarks/` and recorded in
  `context/reference/startup-performance.md` after stages 3 and 7.
- Integration tests isolate configuration, cache, state and account paths as
  AGENTS.md requires.

## Documents to update

- `docs/user-guide/custom-key-bindings.md` and `docs/user-guide/key-bindings.md`
- `context/reference/helix-keymap-v1.md`
- `context/reference/ui-vocabulary.md` where lists and pickers change
- `context/reference/startup-performance.md`
- `context/plans/README.md`
- `AGENTS.md` glossary (done with decision 15)
