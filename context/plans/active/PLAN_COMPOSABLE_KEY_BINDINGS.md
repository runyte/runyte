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
   command. `keymap-export` has no default key and lands at the end of stage 5,
   after the public step grammar opens (decision 42).
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

28. **Pure mode entry (2026-11-27, delegated by the maintainer).** A mode
    step changes mode without moving or collapsing selections. Normal-mode
    selection fitting, including Insert-exit caret and append handling, is a
    separate atomic action. Escape explicitly fits then enters Normal; paste
    enters Normal without fitting. Pure mode steps remain internal in stage 3;
    stage 5 exposes `{ action: mode-enter, arg: <mode> }`. Existing public
    `enter-*-mode` actions remain until then and describe their selection fitting.
29. **Delete and change keep separate cut actions (2026-11-27, delegated by the maintainer).**
    `x d` cuts whole rows into a linewise register; `x c` cuts operative spans
    into a non-linewise register. Preserve two documented decision actions in
    0.5. Revisit explicit linewise arguments in stage 5 rather than inferring
    behavior from a later mode step.
30. **Prompts own completion (2026-11-27, delegated by the maintainer).**
    `search` and `search-regex` remain last-only decision actions; accepting
    enters Select and their descriptors include Mode. Stage 5 supplied search
    arguments allow synchronous composition. Resuming a program after an
    interactive prompt is outside 0.5.
31. **Native media splits follow 0.5 (2026-11-27, delegated by the maintainer).**
    `media-back`, `media-move-up` and `media-move-down` remain documented
    decision actions. Their fit, page and native-selection conditions belong
    to the frontend; splitting them requires an authoritative state design.

32. **Separate atomic cuts (2026-11-27, delegated by the maintainer).**
    `cut-selection` uses delete's whole-row, linewise-register behavior after
    `x`/`X`; `cut-selection-for-change` uses operative spans and a characterwise
    register, keeping the line for insertion. Both preserve mode. Defaults
    `d` and `c` follow them with pure Normal and Insert entry respectively.
    Directory cuts retain move payloads. The old `yank-delete-selection` and
    `yank-change-selection` names are removed with replacement diagnostics.
33. **Paste interprets counts (2026-11-27, delegated by the maintainer).**
    Register and clipboard paste actions insert N copies in one transaction.
    Each range's replacement/insertion decision is made once. Mode splitting
    is postponed by decision 34; existing paste actions retain their mode changes.
    Descriptors use `Interpret`, not repeated invocations. Text and selection
    differences from legacy counted paste are intentional and documented;
    no default-only count policy restores the old incidental mode changes.
    Search extension adds the next/previous match as a new primary selection,
    retaining the existing ranges.

34. **Postpone terminal-sensitive mode splits (2026-11-27, delegated by the maintainer).**
    Do not add buffer-type variants inside programs: they cannot be authored in
    the public configuration format needed by the YAML preset (decision 25),
    and stage 4 would remove them. Keep `y`, `Y`, `p`, `P`, `n`, `N`,
    `Space c p` and `Space c P` as their existing bundled actions for now,
    including inherited directory-tree `N`. Stage 4 gives terminal and
    directory-tree bindings their own modes (decision 19), then completes
    these splits as ordinary bindings. Cuts, `*`, additive search extension,
    and interpreted paste counts proceed independently. Classification tests
    name the bundled actions pending stage 4; that exception list must become
    empty when the postponed splits land.

35. **Settle transient gestures before programs (2026-11-27, delegated by the maintainer).**
    Before the first program step, the input layer applies the existing
    per-action transient-gesture housekeeping. A line-selection gesture ends
    and restores its previous mode unless the first step continues it.
    Conditions observe that settled mode. This is input lifecycle handling,
    not a public housekeeping action. `x v` and `X v` retain their selection
    and enter Select; a second `v` collapses and enters Normal. Collapsing a
    half-open mouse or syntax selection converts to inclusive endpoints first,
    consistently with `collapse-selection`; its resulting caret can be one
    character before the old raw half-open endpoint.

36. **Exhaustive classification (2026-11-27, delegated).** Semantic classes
    are Atomic, Decision with a reason, and ModeAction. Migration status is
    orthogonal: no pending split, or PendingSplit(3), (4), or (5). A pending
    action remains bundled today; its semantic class describes the operation
    after that split. A completed Atomic action cannot change mode as a side
    effect. Explicit exhaustive matches cover editor and colon identities.
    ModeAction includes explicit mode entry and insert-entry caret placement.
    Public enter-mode wrappers await stage 5 (decision 28). The exact stage 3
    set below must be split in the next slice, before the embedded preset;
    stage 3 is not complete until it is empty. Terminal/tree differences and
    line-selection gestures await stage 4 alongside decision 34. Clipboard
    paste chooses image/text and its text mode transition also awaits stage 4.
    Write-quit and write-buffer-close own guarded save continuations, so they
    remain decisions rather than unconditional save/close programs. Tests keep
    the exact pending sets for each stage, and descriptor decision reasons
    appear in discovery and the generated action reference.

37. **Separate no-yank delete and change (2026-11-27, delegated).** Keep two
    mode-preserving decision actions, following decision 32. `delete-selection`
    retains deletion's caret clamp and transient `x`/`X` whole-row semantics.
    `delete-selection-for-change` retains the insertion seam, using insertion
    normalization, and deletes operative spans rather than transient whole rows.
    Their reason is that delete and change place the caret and treat whole rows
    differently. Remove `change-selection` and report the replacement
    `[delete-selection-for-change, mode-enter insert]`; the stage 5 migration-hint
    follow-up supplies the public argument form. Where configured, the no-yank
    delete/change keys append Normal/Insert respectively; neither `Alt-d` nor
    `Alt-c` is a shipped default. Cover deletion of the final character in `ab`
    and transient `x`/`X` selections with parity tests. This pair follows the
    surround/register-replacement slice.

38. **Selection history and whole-row ownership (2026-11-27, delegated).**
    Expansion preserves mode and records the actual mode at invocation. Its
    default appends Select. Shrink remains a Decision: it restores selection,
    coordinate semantics and captured mode from history that a program cannot
    inspect between steps. History matching uses ranges, semantics and syntax
    identity/revision, independent of an explicit intervening mode step.
    Paragraph objects move to PendingSplit(4) with x/X and the line-selection
    actions. Stage 4 separates linewise selection extent from transient mode
    restoration together; there is no paragraph-only workaround. Selection
    actions that produce no result stop the program with informational status.
    Counted selection steps aggregate success across repetitions, so reaching
    a boundary after earlier success still allows the mode tail.

39. **Embedded preset transition (2026-11-27, delegated).** The semantic
    binding, program, namespace and context-action tables, including fast-pane
    alternatives, move to an embedded YAML Runyte preset. UI and pointer default
    tables remain separate (decision 10). `keys.preset` defaults to `runyte`.
    Stage 3 retains the public `bind`/`scopes` table shape. Steps already use the
    final forms `{ action: mode-enter, arg: select }` and
    `{ if: { mode: select }, then: [...], else: [...] }`; existing other
    conditions use their names. These forms are accepted only in embedded
    presets until stage 5; user configuration receives a stage-5 diagnostic.
    Stage 4 changes table shape, and stage 5 removes the step gate.

    Binding metadata is inline in `{ steps: [...], description: ... }`.
    `description` is public. The temporary preset-only metadata is:

    | Field/section | Purpose | Later public availability |
    | --- | --- | --- |
    | `platforms` | Restrict a binding to named build targets (`macos`, `windows`, `linux`); omitted means all targets | No public opening decided |
    | `role` | Primary, fast or compatibility teaching role | No public opening decided |
    | `availability` | Planned/unsupported explanation for discovery | No public opening decided |
    | `target` | Representative identity for a multi-step binding's discovery | No public opening decided |
    | `alias`, `alias-modes` | Advertised alternate spelling and its applicable modes | No public opening decided |
    | `namespaces` | Hint labels, modes, scope and capability of nonexecuting prefixes | No public opening decided |
    | `context-actions` | Context menu labels, mnemonic and row/buffer ownership | No public opening decided |
    | `fast-pane` | Existing optional fast-pane binding overlay | No public opening decided |

    Metadata does not hide additional program steps. The dedicated loader must
    preserve built-in provenance and avoid user scoped admission's dependence
    on the default keymap. Frozen pre-migration fixtures verify the full
    registry, including programs and discovery metadata, for both variants.
    Decision 25's cache trigger is a greater-than-5% regression in the ten-run
    release median readiness measured by `benchmarks/startup.py`, or isolated
    preset compilation exceeding 2 ms. Record before/after measurements in the
    startup register; the register itself defines no budget.

    The profiling amendment makes a disk cache a last resort: first measure
    parsing, validation, descriptor lookup and indexing; index repeated metadata
    lookups and use direct typed deserialization before considering persistence.
    The isolated 2 ms target covers YAML to typed data and programs, excluding
    the pre-existing keymap index. The deciding acceptance metric is stock
    ten-run readiness within 5% of the Rust baseline.
    Isolated YAML + programs is approximately 3 ms, above the 2 ms proxy;
    readiness is within 5%; no cache, by decision. A serial release performance
    test bounds cold parse + program construction at a generous 15 ms median.
    Exhaustive cross-scope validation of the immutable preset belongs in every-preset tests, as it did
    for the Rust tables; schema and program validation remain in the loader.

40. **Identical old and new subsets retain the new meaning.** A configuration
    such as `keys.bind.normal.F6: move-left` is valid in both table shapes.
    Accept it with mode-only semantics rather than adding a discriminator.
    Reject identifiable removed constructs, including `keys.scopes`, with
    migration diagnostics. The guide and 0.5 migration notes explain that
    `bind.normal` applies to normal and modes inheriting it; bindings intended
    across modal views belong in `bind.modal-keys`. `bind.common` also reaches
    Insert and Replace, so only actions valid in every inheriting mode belong
    there (clarified after decision 49). `common`, `modal-keys` and `insert-keys`
    are public, stable shared-mode names in the Runyte preset.
41. **Stage 4 mode boundary.** Stage 4 introduces named binding-mode identity,
    the mode graph, buffer-type starting modes, `inherits` and `unbound-keys`.
    Property-driven editing, the remaining mode schema, `describe-mode` and
    complete custom mode styling remain stage 7 work.
42. **Export follows the public step grammar.** Move `:keymap-export` to the
    end of stage 5, after internal steps become publicly writable. Require
    export/reload parity for effective lookup, programs and discovery.
    Descriptions are exported; preset-only teaching metadata (`role`, `target`,
    `availability`, `alias`, `alias-modes`) is omitted, as stated in the export
    header. `copy-key-binding` adopts the new table shape in stage 4a.

43. **Buffer types map editing roles to named modes.** This is public schema,
    not temporary preset metadata. For example, `markdown: { normal: markdown,
    select: markdown-select, insert: insert, replace: replace }`. A scalar
    `markdown: markdown` supplies the normal role and uses shared select,
    insert and replace modes when present. Normal is the starting role.
    A `mode-enter` argument naming a role resolves through the current buffer
    type; another declared mode name enters that mode directly. An unavailable
    role fails the step and stops the binding. Until stage 7, a mode uses the
    editing semantics of its role in the current buffer type; a submode inherits
    its parent's semantics, defaulting to Normal without a parent. Stage 7
    replaces those semantics with properties, but role indirection remains.
    Terminal activation selects live Insert or review Normal from retained
    terminal state. Validate every role target, parent and graph cycle within
    the same bounded-work limits.

44. **Temporary line-selection mode entry is explicit** (2026-11-27, delegated).
    Defaults use `[select-line, { action: mode-enter, arg: select, temporary: true }]`,
    likewise for `X` and the paragraph objects. Line primitives only set selection
    extent and preserve mode. A temporary entry retains the original return mode
    when a gesture is already active; the next non-continuing program restores
    that mode under decision 35. Retain the named binding identity with its
    editing role. Describe-key displays `temporary: true`. This field remains
    preset-only until stage 5; stage 7 may express it through submode `after-each`
    behavior without changing the gestures.

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
The internal set also includes tree-only `tree-entry-is-root`, needed to keep
`h` at the root from collapsing it while Enter can still toggle it. Internal
programs currently allow 16 total nodes across both branches and four levels
of condition nesting. A skipped branch consumes neither a count nor operands.
The first executed count-accepting action receives the count. During macro
replay, conditional programs reserve repeated work at the selected action,
before that action's effects; an earlier prefix may already have run because
conditions can depend on its effects. Flat programs retain whole-program
preflight. Macro replay actions retain their separate frame and depth bounds.
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

## Classification of existing actions

The exhaustive inventory is `src/command/descriptor/classification.rs`; each
built-in identity is named explicitly, with no fallback. The generated
[Action contracts](../../../docs/user-guide/custom-key-bindings.md) table
lists all 450 current identities and their classification. The descriptor is
also the action browser's source. These are semantic classifications, not
permission, continuation, or count rules. A pending split takes precedence over
its eventual semantic class: it does not certify the current action as atomic.

| Family | Current classification | Remaining work |
| --- | --- | --- |
| Mode-preserving text, selection, navigation, register and configuration operations | Atomic | None |
| Explicit mode entry and insert-entry caret placement | ModeAction | Public enter-mode wrappers await stage 5 |
| Per-range paste, both cut operations, prompted search | Decision, with descriptor reason | Paste mode split in stage 4; search arguments in stage 5 |
| Native media navigation | Decision, with native-state reason | Decision 31 retains it through 0.5 |
| Clipboard image/text dispatch | Decision | Text fallback mode split in stage 4 |
| Save-and-close commands | Decision | Retain guarded save continuation and success-dependent close |
| View/pane activation, Git workflow destinations and completion | Decision | Mode belongs to guarded destination/continuation; no unconditional mode tail |
| Bundled operations listed below | Pending split; underlying semantic class retained separately | Must empty each exact set before its stage is complete |

Surround add/delete/replace now preserve mode; their default `m s`, `m d`,
and `m r` programs append explicit Normal entry after successful operands.
`replace-with-register` already preserved mode on successful replacement;
its descriptor and refusal/gesture paths now consistently reflect that contract.
These four entries are complete. Decision 37 is also implemented: the no-yank
delete pair preserves mode, with distinct caret and transient-row semantics,
and `change-selection` reports its replacement program. The selection-family
slice completes the remaining stage 3 classifications under decision 38.

### Pending stage 3 splits

None. Selection primitives preserve mode and default programs name mode entry.
Shrink is a retained history-restoring decision; paragraphs join the whole-row
ownership split in stage 4. The final stage 3 slice supplies the embedded YAML
preset described below.

### Pending stage 4 splits

None.

Stage 4c separates terminal paste and review search, makes document paste and
repeat-search mode-preserving, and replaces bundled yank actions with programs.
Terminal **Y** now copies whole review rows; it was previously refused. Stage 4c2
separates terminal undo-paste and live-input entry; its pending dispatch
inventory is empty. Normal and Select entry retain their shared meanings.
Directory paste now has its own entry-payload decision with text-register
fallback, while shared before/after placement retains transfer ownership. Tree
search has three dedicated actions and ordinary bindings, including **N**.

### Pending stage 5 splits

`enter-normal-mode`, `enter-insert-mode`, `enter-replace-mode`, `enter-select-mode`.

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
   configuration shapes and migrate the Runyte preset to the new shape.
5. **Step syntax, arguments and conditionals.** Update removed-action migration
   hints to use the writable mode-argument form and test that every suggested
   replacement compiles through the public configuration compiler.
   The `{ action:, arg: }` step
   form with `count`, `register` and `extend` fields (decisions 17 and 18),
   arguments that replace operands and prompts, `insert-text`, and
   conditional steps with the documented condition set. Add mode-flow validation:
   validate each step against the mode it will run in, derived from preceding
   mode steps and independently for each conditional branch, rather than the
   binding's starting mode. For example, `[enter-insert-mode, insert-newline]`
   becomes valid. Add `:keymap-export` (no default key) with export/reload
   parity after lifting the public step gate (decision 42).
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

### Stage 4 implementation slices

Each slice keeps behavior coverage, platform parity, desktop checks and review
with its implementation. This is the order for the remaining stage 4 work:

- **4a — Named binding modes and table migration.** One inheritance chain,
  buffer-type starting modes, implicit declarations, `inherits`, `unbound-keys`
  and `common`. Migrate the embedded preset to block YAML, preserving platform
  applicability, fast-pane alternatives and discovery metadata. Update
  `copy-key-binding`, configuration diagnostics and migration notes. Keep
  Command and List UI ownership distinct. Measure startup again.
- **4b — Whole-line selection extent.** Move linewise extent into selection
  state and separate it from temporary mode restoration for line and paragraph
  primitives, including selection history, undo, registers and terminal review.
- **4c — Terminal meanings and programs.** Split operations with distinct
  terminal meanings and author their explicit programs and mode tails. Retain
  shared operations where their meaning is unchanged.
- **4c2 — Remaining terminal meanings.** Terminal undo-paste and live-input
  entry have distinct identities; Normal/Select entry remains shared with view
  prose. Document input aliases are refused. The pending inventory is empty.
- **4d — Remaining directory, tree and clipboard splits.** Include inherited
  tree `N` and preserve clipboard image/text decisions. Leave the stage 4
  pending-split classification empty.
- **4e — Family names.** Migrate action descriptors, preset, guide and tests
  together. Diagnose old names with their replacements; do not keep aliases.

Stage 4a implements decision 43's exact per-buffer-type role maps, including
refusal before side effects when a role is absent. Named identity follows
input, discovery, native preview projection, temporary restoration and reload;
Command/List ownership remains separate. The preset parity tests cover four
platforms and both fast-pane variants. Linux readiness remains within the 5%
budget after sharing identical immutable indexes; measurements are recorded in
`context/reference/startup-performance.md`.

Stage 4b makes whole-line extent selection state. Line and paragraph actions
preserve mode; default programs add decision 44's temporary entry. Selection and
syntax-expansion history restore extent. Text undo retains its existing contract:
transactions map live selections, so a collapsed post-edit selection does not
recover a historical extent. Terminal review and register/clipboard consumers
read the same retained extent. Coordinate remapping preserves it; semantic range
changes reset it.

Export follows stage 5, not a sixth stage 4 slice (decision 42).

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

### Atomic migration increments

The first atomic increment introduces internal pure mode entry and
`fit-selection-for-normal-mode`, rewrites Escape, and separates the mode
steps from `select-all`, `collapse-selection`, `toggle-case` and `replace-char`.
Public `enter-*-mode` actions retain their existing behavior. Current-format
exports refuse internal mode programs rather than emitting an incomplete
binding; commented examples omit them, independently of default removal.

The remaining text, yank/paste, search and conditional default rewrites and
exhaustive atomic/decision classification remain stage 3b work. Decisions
29–31 specify the intentional decision actions rather than completed splits.

The next increment splits cuts and `*`, adds additive search extension, and
interprets paste counts in one transaction. Stage 4 completes the postponed
splits listed in decision 34. Public configuration cannot yet express internal
pure mode steps, so exports continue to refuse those programs.

The internal-conditional increment rewrites `v`, `Space m m` and directory-tree
`h`/Left, `l`/Right, Enter, `v` and `s` as programs. Select entry and default
macro recording are start-only actions; tree collapse, parent selection,
expansion and first-child selection are separate actions, and tree file opens
no longer toggle directories. Discovery renders both branches and scoped
admission includes their leaf actions. Public conditional authoring remains
stage 5; exhaustive classification and the embedded YAML preset remain the
next stage 3b slices.

### Embedded preset completion

The final stage 3 slice moves all semantic bindings, programs, namespaces and
context actions, including fast-pane alternatives, to
`src/keymap/presets/runyte.yaml`. Default initialization compiles typed YAML once
per process; `keys.preset` selects the embedded `runyte` preset and unknown
names report a non-fatal fallback. The public description wrapper is accepted,
while pure mode entry and conditional authoring retain the stage 5 gate.
Frozen #151 registry and preferred-spelling fixtures cover both variants.
Decision 39's profiling amendment is recorded in the startup register: optimized
stock readiness passes the deciding 5% budget without a disk cache. Stage 4
configuration consolidation and stage 5 public programs remain outstanding.

### Decision 45 — Validate mode-entry roles when loading

For each buffer type, validate every mode-entry role target in effective bindings
reachable from its role map, including inherited bindings, conditional branches,
and named destinations. Respect overrides and removals. Report the buffer type,
active mode, key and missing role; reject the incompatible graph and restore the
preset graph while retaining valid key edits. Embedded presets pass the same
check. Terminal paste programs therefore cannot deliver text with an unresolved
Insert tail. This is the role-resolution part of stage 5 validation; sequential
mode tracking remains in stage 5. No runtime paste-tail preflight is used.

### Decision 46 — Uniform clipboard paste modes

Clipboard paste preserves mode for both image and text content. Normal/Select
bindings append explicit Normal entry; Insert/Replace bindings retain their role.
Native clipboard input uses the same explicit role policy independently of key
remapping. This deliberately changes Insert/Replace text fallback, which formerly
returned to Normal. Format selection remains one image-first clipboard decision;
empty, failed and refused pastes do not run a mode tail.

### Decision 47 — Subject-first family names

Stage 4e migrates the following 66 action spellings. Old names are refused
with replacement diagnostics in bindings, sequences and named commands.
Seven matching colon spellings migrate with the actions; other colon
spellings remain unchanged. The preset, discovery and current documentation
use the replacements. Internal command identities retain their behavior.

Core text and selection verbs, including syntax text objects and motions,
`match-bracket`, `fit-selection-for-normal-mode` and `replace-with-register`,
keep their names. Bare `search` and already subject-first families also stay.
The four `enter-*-mode` actions remain PendingSplit(5): stage 5 replaces them
with argument-bearing `mode-enter` steps rather than introducing temporary
names in this slice.

| Old action | Replacement |
| --- | --- |
| `select-register` | `register-select` |
| `record-macro` | `macro-record` |
| `record-default-macro` | `macro-record-default` |
| `stop-macro-recording` | `macro-stop-recording` |
| `replay-macro` | `macro-replay` |
| `replay-default-macro` | `macro-replay-default` |
| `list-macros` | `macro-list` |
| `global-search` | `search-workspace` |
| `global-search-regex` | `search-workspace-regex` |
| `open-workspace-search-result` | `search-open-workspace-result` |
| `open-fuzzy-grep` | `search-open-fuzzy-grep` |
| `expand-syntax-selection` | `syntax-expand-selection` |
| `shrink-syntax-selection` | `syntax-shrink-selection` |
| `document-outline` | `syntax-document-outline` |
| `toggle-syntax-fold` | `syntax-toggle-fold` |
| `fold-all-syntax` | `syntax-fold-all` |
| `unfold-all-syntax` | `syntax-unfold-all` |
| `open-explorer` | `directory-open` |
| `open-working-directory-explorer` | `directory-open-working-directory` |
| `open-directory-file-picker` | `directory-open-file-picker` |
| `open-directory-fuzzy-grep` | `directory-open-fuzzy-grep` |
| `open-directory-entry` | `directory-open-entry` |
| `open-parent-directory` | `directory-open-parent` |
| `refresh-directory` | `directory-refresh` |
| `toggle-hidden-files` | `directory-toggle-hidden-files` |
| `toggle-directory-details` | `directory-toggle-details` |
| `choose-explorer-order` | `directory-choose-order` |
| `open-explorer-system` | `directory-open-system` |
| `open-explorer-finder` | `directory-open-finder` |
| `open-explorer-workspace` | `directory-open-workspace` |
| `toggle-directory-tree` | `directory-tree-toggle` |
| `focus-directory-tree` | `directory-tree-focus` |
| `open-changed-file` | `git-open-changed-file` |
| `stage-all-changed-files` | `git-stage-all-changed-files` |
| `checkout-branch` | `git-checkout-branch` |
| `create-branch` | `git-create-branch` |
| `delete-branch` | `git-delete-branch` |
| `open-git-conflict` | `git-open-conflict` |
| `pull-branch` | `git-pull-branch` |
| `push-branch` | `git-push-branch` |
| `open-worktree` | `git-open-worktree` |
| `create-worktree` | `git-create-worktree` |
| `create-new-worktree` | `git-create-new-worktree` |
| `remove-worktree` | `git-remove-worktree` |
| `next-git-network-page` | `git-network-next-page` |
| `previous-git-network-page` | `git-network-previous-page` |
| `next-git-log-page` | `git-log-next-page` |
| `previous-git-log-page` | `git-log-previous-page` |
| `open-git-commit` | `git-open-commit` |
| `open-terminal` | `terminal-open` |
| `open-terminal-file-directory` | `terminal-open-file-directory` |
| `open-terminal-directory-root` | `terminal-open-directory-root` |
| `open-terminal-selected-directory` | `terminal-open-selected-directory` |
| `open-terminal-session-directory` | `terminal-open-session-directory` |
| `open-terminal-list` | `terminal-open-list` |
| `show-terminal` | `terminal-show` |
| `rename-terminal` | `terminal-rename` |
| `leave-terminal` | `terminal-leave` |
| `copy-terminal-output` | `terminal-copy-output` |
| `send-to-terminal` | `terminal-send` |
| `previous-session` | `session-previous` |
| `next-running-session` | `session-next-running` |
| `previous-running-session` | `session-previous-running` |
| `open-session-directory` | `session-open-directory` |
| `open-explorer-session` | `session-open-explorer` |
| `open-session-destinations` | `session-open-destinations` |

### Decision 48 — Public program contracts and stage 5 slices

The public step syntax is a bare action name or `{ action: <name>, arg:
<argument> }`. Stage 5a rejects the former `{ command:, argument: }` form and
`{ name: arg }` shorthand with a migration diagnostic. Copy and export emit
only the new syntax. Embedded presets and configured bindings share typed
step decoding and action resolution; preset-only mode/conditional syntax
remains gated until mode-flow validation is ready.

For `{ mode: <name> }`, `normal`, `select`, `insert` and `replace` resolve
through the current buffer type's role map, including collisions with declared
mode names. Other names address an exact declared mode. This matches
`mode-enter` role resolution.

Without a count/register step field, the first eligible executed step consumes
the pending prefix. Fixed values and `none` do not consume it; `inherit`
reuses the original captured value. `register: none` selects the unnamed
register. `register-empty` sees the pending selected register until consumed,
then the unnamed register. Skipped conditional branches consume nothing.

Export preserves normalized executable lookup, programs, arguments, options,
descriptions and the mode graph, not representative targets or alias
provenance. Planned and unsupported bindings are omitted, with that omission
stated in the export header; export does not invent a public representation
for them.

Implementation order (each slice reviewed separately):

1. **5a — Shared typed action syntax.** Bare names and `{ action, arg }`,
   existing colon arguments, descriptions, strict fields/types, migration
   diagnostics and copy round trips. Reject count/register/extend for now.
2. **5b — Mode-flow validation and public mode/conditional steps together.**
   Resolve roles per buffer type, track sequential modes and branch joins,
   inheritance/removals, temporary restoration and dynamic effects such as
   syntax-shrink. Lift the embedded-only gate only with this validation.
3. **5c — Supplied operands and prompts.** Publish the supported argument
   inventory and `insert-text`; do not resume interactive prompts mid-program.
4. **5d — Count, register and extend fields.** Apply the consumption contract
   above, including conditional branches.
5. **5e — Remaining decision 14 conditions.** Native media splits remain
   deferred beyond 0.5.
6. **5f — Retire the four enter-mode wrappers.** Supply compilable migration
   hints using public mode steps.
7. **5g — Effective export.** Implement the normalized contract above last.

`keep-count` and other mode properties remain stage 7; saved macros remain
stage 6. Each implementation slice receives Astra High review rounds, core
and affected desktop checks, and coverage verification. Compiler changes also
receive the alternating ARM preset startup probe.

### Decision 49 — Effective mode paths and condition matching

Stage 5b admits a program only when every step is valid on every reachable
effective buffer-type and named-mode path, after inheritance, replacements and
removals. Conditional branches join their successful output states before the
remaining steps are checked. An Insert-only tail inherited by a Normal child
is a load error identifying the buffer type, active mode, key and step; bind
such programs in their specific mode. Read-only state, terminal readiness and
language-server availability remain runtime capability checks.

History-restoring steps, including syntax shrink and selection undo/redo,
join every named mode they can restore. A mode-specific tail therefore needs
an explicit `mode-enter` to narrow those possibilities first.

A role condition such as `{ mode: select }` tests the current mode's editing
role, including a submode inheriting that role's semantics (decision 43).
It does not compare the exact role-target identity: the preset's `v` still
leaves a Select-like submode. A declared non-role name instead matches the
exact active named mode. This refines decision 48's role-condition wording;
explicit entry still resolves role names through the buffer-type role map.

The stage 5b cost gate compares the native-preview fixture's real
`:config-reload` against dev `d564c471` on the same ARM runner. In release
builds, changed-key and unchanged reloads must remain within 25% or 20 ms of
dev, whichever allowance is larger. Release editing readiness retains the
5% bound. Report paired debug and release measurements on x86 and ARM;
preset initialization alone does not establish configured-reload cost.
Caching may reuse validated work, but must preserve the effective-path
admission contract above, including paths removed by configuration.

### Decision 50 — Temporary gestures belong to their view

Changing pane focus, active buffer, or active buffer type ends a temporary
mode gesture and discards its return mode. This includes directory-tree focus,
Help and generated pages. The destination uses its ordinary activation rule;
role identities resolve against the destination buffer type. Same-view
restoration remains unchanged. This extends the focus ownership rule for
pending programs from #141 and fixes a stage 4 regression where a directory
return mode could replace Help's key table. Scope-specific commands exposed
through stale return identities are bugs, not runtime capability exceptions.


### Decision 51 — Supplied argument inventory and literal insertion

Stage 5c accepts one Unicode scalar for find-next/previous/till-char,
replace-char, surround-add/delete, register-select and macro-record/replay;
surround-replace accepts the old and new scalars. Search/search-regex,
terminal search, directory-tree search, keep/remove-matching-selections,
join-selections and workspace search accept their prompt text. Workspace search
remains asynchronous and last-only. Existing palette arguments remain supported.
Captured or multi-step Git, native confirmation, settings, LSP rename and tree
filesystem prompts have no generic bypass.

`insert-text` inserts a nonempty literal (at most 4096 bytes, allowing newline
and tab) at every range head without deleting the selection. Every resulting
caret is after its insertion. It is one transaction under program undo, valid
in all four editing roles, with read-only refusal and no automatic indentation,
pairing or mode change. Replacement composes an explicit deletion with insertion.


### Decision 52 — Explicit prefix-setting steps (2026-11-28)

Binding-entry capture in decisions 18 and 48 preserves what was selected before
the binding. An explicit `register-select` step updates only the pending register
for subsequent steps without a register field. `register: inherit` continues to
reuse the original captured register; fixed and `none` overrides are unaffected.
Thus `[{action: register-select, arg: a}, yank]` still writes register `a`.
The captured and pending values survive character-operand waits independently.
A future explicit count-setting action would update only the pending count in
the same way. Stage 5d tests cover entry prefixes, explicit selection, inheritance,
and operand waits together.
