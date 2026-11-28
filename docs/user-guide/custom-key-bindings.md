## Custom key bindings

The `keys` section configures keyboard actions and pane gestures:

| Key | What it does |
| --- | --- |
| `keys.preset` | Selects an embedded semantic preset; currently `runyte` (the default). |
| `keys.leader` | Replaces `Space` as the leader prefix. |
| `keys.window` | Replaces `Ctrl-w` as the window prefix. |
| `keys.rebind` | Moves bindings Runyte already ships to other keys. |
| `keys.bind` | Assigns named actions to keys, per mode. Also removes bindings. |
| `keys.defaults` | `false` starts `bind` from an empty keyboard instead of Runyte's. See [Without the built-in keys](custom-key-bindings.md#without-the-built-in-keys). |
| `keys.pointer` | Replaces editor, pane, media, and preview pointer gestures. |
| `keys.ui` | Replaces actions in prompts, menus, overlays, confirmations, and native controls. |
| `modes` | Declares mode inheritance and the policy for unbound keys. |
| `buffer-types` | Maps each buffer type to its starting mode and editing roles. |

- `keys` is read from the configuration file and from
  [`keybinds.d/`](configuration.md#themes-and-key-bindings-in-their-own-files) beside it. The
  `[config]` page never writes it. Run `:config-reload` after editing.
- Plugin commands are bound separately, with `plugins[].bindings` in the
  [plugin guide](../plugins.md).
- `:help key-actions` lists every bindable action inside the editor. The same
  list is in [Bindable action reference](custom-key-bindings.md#bindable-action-reference).

### Quick examples

**Save with `Ctrl-s` in every mode:**

```yaml
keys:
  bind:
    normal:
      Ctrl-s: save
    select:
      Ctrl-s: save
    insert:
      Ctrl-s: save
    replace:
      Ctrl-s: save
```

**Use `Ctrl-x` as the leader:**

```yaml
keys:
  leader: Ctrl-x # Space f becomes Ctrl-x f, and so on
```

**Move the Git menu from `Space g` to `Space G`:**

```yaml
keys:
  rebind:
    Space g: Leader G
```

**Open the changed-file list with `F9`:**

```yaml
keys:
  bind:
    normal:
      F9: git-status
```

Bindings may also provide an authored discovery description:

```yaml
keys:
  preset: runyte
  bind:
    normal:
      F6:
        steps: [select-all, yank]
        description: Copy the whole document
```

Unknown preset names report a configuration error and keep the Runyte base.
Preset names never identify files. The same mode-entry and conditional step
syntax is available in embedded presets and configured bindings.

### Changing the leader and window keys

```yaml
keys:
  leader: Ctrl-x
  window: Ctrl-a
```

- Each takes exactly one key.
- `window` cannot be an unmodified character, including `Space`. It is
  captured in Insert, Replace, and Terminal Insert modes, so a plain
  character would make that character impossible to type.

**A new window prefix** moves the whole window menu in Normal, Insert, and
Terminal Insert modes.

- The old `Ctrl-w` is then passed to terminal programs.
- The new key is taken from them. `Ctrl-a`, for example, is tmux's usual
  prefix and readline's beginning-of-line.

**A new leader** also becomes the key that dismisses modal lists, choice
popups, action menus, ordinary confirmations, and an empty picker.

- A bare `Space` becomes ordinary input there.
- Confirmations that ask you to type exact text, such as a branch name or a
  path, still treat every printable key, including the leader, as text.

### Moving default bindings

`keys.rebind` maps a default key sequence (left) to a new one (right).

```yaml
keys:
  leader: Ctrl-x
  window: Ctrl-a
  rebind:
    Space g: Leader G # the whole Git menu moves
    Space g l: Leader G q # ...but Git log goes here instead
    Ctrl-w x: Window e
    Space e: Space
    ",": F12
```

**Writing the two sides:**

| Side | What it accepts |
| --- | --- |
| Left | A default spelling only: a binding, alias, or prefix Runyte ships. Write `Space` and `Ctrl-w`, never `Leader` or `Window`. |
| Right | Any key sequence. `Leader` and `Window` mean the configured prefixes; literal `Space` and `Ctrl-w` mean those physical keys. |

**How rules combine:**

- All rules apply at once. File order does not matter.
- The longest matching left side wins. Above, `Space g d` becomes
  `Ctrl-x G d`, while `Space g l` becomes `Ctrl-x G q`.
- A narrower rule may deliberately move a key out of the namespace a broader
  rule chose.

**What `rebind` cannot do** (use `keys.bind` instead):

- remove a binding,
- bind a command that has no default key,
- move most direct single-key editing bindings.

### Named modes and buffer types

A binding lives in `keys.bind.<mode>`. Lookup starts in the active named mode,
then follows its single `inherits` parent until a binding is found. There is no
second global or view lookup. A binding table implicitly declares its mode,
even when the table is empty. The Runyte preset exposes three stable shared-mode
names:

- `common`: the parent shared by editing and specialized view modes, including
  Insert and Replace. An action here must be valid in every inheriting mode.
- `modal-keys`: shared by the preset's Normal/Select families and modal views.
  Use it for former global Normal bindings; it does not supply text-entry keys.
- `insert-keys`: shared by the preset's Insert/Replace families, including
  terminal input modes. Actions must be valid in all those destinations.

They are ordinary inherited modes: a child may override or remove a key, and a
custom mode only receives their bindings if its inheritance reaches them.
For example:

```yaml
keys:
  bind:
    common:
      Ctrl-s: save
    modal-keys:
      F6: yank
    insert-keys:
      Alt-Left: move-left
```

Use a narrower named mode when an action is not valid everywhere that inherits
the shared table. For example, `insert-newline` belongs in a text Insert mode,
not `common` or the terminal's input table.

```yaml
modes:
  writing:
    inherits: normal
    unbound-keys: report
buffer-types:
  markdown:
    normal: writing
    select: markdown-select
    insert: insert
    replace: replace
keys:
  bind:
    common:
      Ctrl-s: save
    writing:
      j: move-up
      k: null
    list:
      F9: list-next
      Ctrl-n: null
    session-manager:
      F8: list-previous
```

An exact `null` suppresses that key throughout the inherited chain. It does
not reveal the parent binding or remove longer sequences beginning with the
same key. A child binding replaces the inherited key and its longer sequences.
Only one parent is supported; cycles, unknown parents and unknown role targets
are rejected. The effective graph has at most 256 modes, names have at most
64 ASCII letters, digits, hyphens or underscores, and inheritance has at most
64 levels.

`buffer-types` entries map `normal`, `select`, `insert` and `replace` roles to
named modes. The `normal` role supplies the starting mode. The scalar shorthand
`markdown: writing` supplies that normal role and uses shared `select`, `insert`
and `replace` modes when they exist. An explicit mapping supplies exactly the
listed roles. Entering an omitted role is unavailable and stops the binding.
Internal mode-entry steps resolve role names through this mapping; other declared
names enter that mode directly. Until editing properties arrive, a named mode
uses the semantics of its mapped role, or the nearest mapped ancestor; an
unmapped root uses Normal semantics. Terminal activation preserves live input
or review: its Insert role is live input and its Normal role is review.

`unbound-keys` accepts `report`, `pass-to-buffer` and `overwrite`. It defaults
to `report`; the built-in Insert and Replace modes declare the corresponding
text-input policies. Properties belong to the mode; only key lookup inherits.
Mode declarations merge property by property with the preset, while a supplied
buffer-type role mapping replaces that type's entire mapping. Reload retains a
submode by name only while its editing role stays the same; otherwise it returns
to the current buffer type's mapped role. If focus, reload or restoration carries
a role the destination omits, it falls back to Normal. The UI-owned `list` and
`session-manager` tables cannot be editing-role targets or parents of editor
modes. A shared table may contain an action
available in only some inheriting roles; execution still checks availability
in the active role before that action runs.

The preset's `normal` inherits shared modal keys, and `select` inherits `normal`.
Markdown inherits `normal`; other special views have their own named modes.
For example, `keys.bind.markdown` changes Markdown normal and its inheriting
Select mode, while `keys.bind.markdown-select` changes Select alone. Terminal
input uses `keys.bind.terminal-insert`:

```yaml
keys:
  bind:
    terminal-insert:
      F6: enter-normal-mode
      Ctrl-4: null
      'Ctrl-\': null
      'F7 h': focus-window-left
```

Ordinary terminal keys still reach the child. An assigned action claims its
first key and following sequence. A null removes the exact binding; surviving
longer bindings retain their prefix. Built-in terminal input routing remains
until the terminal command split. Terminal review and special views still
apply their runtime action availability checks.

Result lists use `keys.bind.list`; session-manager overrides use
`keys.bind.session-manager`. List bindings have one key and support `list-next`,
`list-previous`, `list-page-down`, `list-page-up`, `list-first`, `list-last`,
`list-toggle-preview`, `list-clear-filter`, `list-close-target`, `list-close`,
`list-accept`, `list-backspace` and `list-open-actions`. Exact modifier spellings
are independent. Numbered session choices and nested action menus retain their
own input ownership. `keys.ui.list-close` configures close confirmations.

Bindings accept action names, action lists, argument mappings, mode entry,
and conditional branches.
Prefixes have at most eight keys. Prompt, menu, overlay and native controls
remain under `keys.ui`; they are not named editor modes.

### Upgrading to 0.5

`keys.scopes` has been removed and produces a diagnostic. Move each view table
to its named mode under `keys.bind`: for example, `scopes.markdown.normal`
becomes `bind.markdown`, `scopes.terminal.insert` becomes `bind.terminal-insert`,
`scopes.global.list` becomes `bind.list`, and `scopes.session-manager.list`
becomes `bind.session-manager`.

Existing `keys.bind.normal` syntax remains valid with new semantics: it changes
text Normal and modes that explicitly inherit it, including Markdown. It no
longer overlays every special view. Move former 0.4 global Normal bindings to
`keys.bind.modal-keys`. Use `keys.bind.common` only for actions valid in every
inheriting mode, including Insert and Replace. Check Select inheritance when translating old separate
Normal and Select tables. `copy-key-binding` emits the named table that supplies
the inspected key. Full preset export follows the public program syntax work.

### Action name migration

In 0.5, actions belonging to a subject family use subject-first names.
Replace old names in key bindings, sequences and `{ action: ... }` entries
using this table. Old names report their replacement; they are not aliases.
The same change applies to colon commands whose spelling matches a row.
Other colon spellings, such as `:terminal` and `:terminal-file-directory`,
remain available. Default keys and action behavior do not change.

Core text and selection operations, including syntax text objects and
motions, keep their names. The four `enter-*-mode` actions also retain their
current names pending the mode-argument migration.

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

### Binding actions

Completion popup actions can be assigned separately from Insert/Replace editing:

```yaml
keys:
  ui:
    completion:
      next: [Ctrl-j]
      previous: [Ctrl-k]
      accept: [Ctrl-y]
```

Each action list replaces all its default bindings. `[]` removes them. Omitted
actions keep defaults. UI actions are independent of `keys.defaults`.
Configuration fragments merge by owner and action. A conflict with another
effective action restores the owner's defaults with a diagnostic.

| Completion action | Default keys | Effect |
| --- | --- | --- |
| `next` | Down, Ctrl-n | Select the next candidate |
| `previous` | Up, Ctrl-p | Select the previous candidate |
| `accept` | Tab | Insert the selected candidate |
| `cancel` | Ctrl-c | Dismiss and consume the key |
| `cancel-or-continue` | Esc | Dismiss; word completion and directory buffers pass the key to ordinary editing |
| `dismiss-and-continue` | Enter | Dismiss and pass the key to ordinary editing |

The continuation actions pass the **actual pressed key** once, including its
modifiers. If remapped to F9, they run the underlying F9 binding, not the old
Esc/Enter binding. They never synthesize input. UI bindings currently accept
single strokes, with up to 32 aliases per action. Configured modifiers match
exactly using the same character Shift normalization as editor bindings.
Unchanged defaults keep their existing handling of additional modifiers.

The `context-actions`, `path-actions`, and `program-actions` owners support `next`, `previous`,
`accept`, and `cancel`. Context and path actions default to Down/j,
Up/k/Shift-Tab, Enter, and Tab/Esc/Ctrl-c respectively. Program actions use
Down/Ctrl-n and Up/Ctrl-p/Shift-Tab for navigation, Enter to accept, and
Tab/Esc/Ctrl-c to cancel. The `path` popup supports `open-actions` (Tab) and
`cancel` (Esc/Ctrl-c). These menu owners also inherit the effective leader as a
cancel key. Reassigning or removing `cancel` removes that leader binding too.

The nested `buffer-actions`, `terminal-actions`, and `session-actions` owners
support `next`, `previous`, `accept`, `back`, and `cancel`. Their navigation uses
Down/Ctrl-n and Up/Ctrl-p/Shift-Tab. Enter accepts, Tab goes back, and Esc/Ctrl-c
cancels. Both back and cancel close the menu. They are separate actions because
back, like acceptance, refuses a stale selected row. Cancel can always dismiss
that stale menu. This distinction follows the actions when remapped. These
owners also inherit leader cancellation. Replacing cancel leaves back available.
Replace or remove both actions to change every dismissal key.

For terminal and session actions requiring a second confirmation, the
configured accept key both arms and confirms. Navigation disarms the
confirmation. An unbound key does not. Acceptance still revalidates the
captured target and requires the current presented frame where that gate
applies.

The `plugin-manager-actions` owner provides a transparent `cancel` action that
returns from the plugin action list to the manager, preserving its filter and
selection. Escape, Ctrl-c, and the empty-filter leader are its defaults. Other
input continues through `global.list`. This return action takes precedence over
scoped bindings in that list. Hints and acceptance authority use the same
order. Replacing or removing return bindings exposes any scoped action
underneath. For example, removing this owner's cancel makes Escape run
`list-close`, unless you also change Escape in `global.list`. Removing its
inherited leader restores literal filter input without an implicit Escape
translation.

The `confirmation` owner supports `accept` and `cancel` for file and directory
reloads, buffer and Git discard, and pull/rebase confirmation. Defaults are Enter
and Esc/Ctrl-c. `stash-confirmation` has the same actions but defaults to Enter
and Esc. Ctrl-c remains inert there. Both inherit leader cancellation only where
the existing overlay allows it (for example, an underlying nonempty filter
suppresses it). Replacing `cancel` also replaces inherited leader cancellation.
A remapped acceptance still revalidates the captured file, repository, or other
target, and requires the current presented frame wherever that check applies.

The `text-confirmation` owner handles branch switching/deletion and worktree
removal. It supports `accept`, `cancel`, `backspace`, `delete`, `move-left`,
`move-right`, `move-start`, and `move-end`. Editing defaults are Backspace,
Delete, Left, Right, Home, and End. Prompt-style Ctrl-h/Ctrl-a aliases are not
inherited. Text and pasted names remain structural input. Remapping acceptance
does not bypass an exact-name requirement or change the authorization required
by the captured operation. Untyped branch/worktree decisions use the same owner,
but their editing actions remain inactive.

The `filesystem-confirmation` owner supports `accept`, `cancel`, `next`,
`previous`, `page-up`, `page-down`, `first`, and `last`. Enter applies the
reviewed plan with trash deletion. Esc/Ctrl-c cancel. Navigation defaults are
Down/Ctrl-n, Up/Ctrl-p, PageDown/Ctrl-d, PageUp/Ctrl-u, Home, and End. Plain
`P` remains the structural permanent-deletion choice. Shift-P does not confirm
it. An inherited leader takes precedence over that choice, and the hint
reflects that precedence. Plugin-owned plans keep their host execution and
captured approval checks.

The `tree-destination` owner supports `accept` (Enter), `cancel` (Esc), and
`backspace`. Plain pane digits remain structural and still select immediately
when they identify one captured pane. `tree-delete` supports `cancel` (Enter or
Esc). The y/Y and n/N keys remain structural yes/no choices. Default
tree-delete keys accept Shift/Meta/Hyper but ignore Control/Alt/Super.
Configured cancellation matches its exact authored modifiers. Neither tree
owner adds a Ctrl-c alias. Both preserve conditional leader cancellation and
captured targets.

Merge-review controls use `keys.bind.git-merge-review`, `git-merge-detail`,
and `git-merge-input`. Acknowledgment editing uses `ui.merge-acknowledgment`
with `backspace`, `delete`, `move-left`, and `move-right`. Literal text remains
structural. Editing preserves the pending scoped key sequence and the existing
cursor behavior. Removing an editing alias permits that key to be assigned in
`git-merge-input`. Configurations cannot advertise a scoped action hidden by
an active editing binding.

`ui.merge-prefix.reset-and-continue` defaults to Escape with any modifiers.
It clears the pending review sequence, then resolves the same input through
the scoped keymap. Other default cancellation aliases do not interrupt an
unfinished sequence. Replacing or removing this action permits Escape inside
configured review sequences. A reset key owned by the scoped input layer cannot be used after the
first stroke of a sequence. Literal text and acknowledgment editing take precedence
over prefix reset. Outside acknowledgment editing, the leader keeps
its existing alias to the scoped Escape binding.

Plugin input uses separate owners for each interaction:

| Owner | Actions |
| --- | --- |
| `plugin-confirmation` | `accept`, `cancel` |
| `plugin-picker` | `accept`, `cancel`, `next`, `previous`, `backspace` |
| `plugin-form` | `accept`, `cancel`, `next`, `previous`, `backspace`, `delete`, `move-left`, `move-right`, `move-start`, `move-end` |
| `plugin-choice` | `accept`, `cancel`, `next`, `previous`, `move-left`, `move-right`, `toggle-selection` |
| `plugin-path-completion` | `accept`, `accept-or-continue`, `next`, `previous` |

These owners do not inherit leader cancellation. Form text and picker filters
remain literal input. Choice fields use plain Left/Right and Space by default.
Space is the configurable `toggle-selection` action there. Text fields keep
Space as text. Picker Backspace removes one character. Form editing keeps
its grapheme-aware cursor and deletion behavior.

Path completion takes precedence only while candidates are available. `accept`
defaults to Tab. `accept-or-continue` defaults to Enter and completes an
unfinished path, otherwise allowing the form to handle Enter. Completion
Up/Down cycle candidates. Modified navigation falls through to the form.
Repeating Tab may complete a path, while repeating conditional completion or
submission cannot submit a form. Remapped submission keeps fresh-input,
presented-frame, field-validation, and captured-context checks. Completion
cannot lend submission authority to another input owner.

The `provider-reload` owner supports `next`, `previous`, `accept`, and `cancel`.
`provider-overwrite` supports `accept` and `cancel`. Both keep fresh physical
approval and captured document checks. Reload defaults to plain Down/Tab and
Up, exact Ctrl-n/Ctrl-p, and any-modifier BackTab. Shift-Tab remains inert.
Enter accepts, Esc or exact Ctrl-c cancels, and the leader also cancels.
When Enter is the leader, reload cancels while overwrite accepts, preserving
its original precedence. Repeated acceptance cannot approve either dialog.

The `context-review` owner controls permission-review navigation: `next`,
`previous`, `toggle-selection`, `accept`, `cancel`, `first`, `last`, `page-up`,
`page-down`, `half-page-up`, and `half-page-down`. Choice defaults are Down, Up,
Tab/Shift-Tab/BackTab, Enter, and Esc. Scrolling inherits the approved normal-mode
editor motions, with plain Home/End fallbacks. Configuring a scrolling action
replaces its inherited motions as well. An empty list disables them. Removed
motion sequences remain consumed so they cannot become permission choices.
The single-key permission choices (1/2, r, x, n) remain structural. Remapped
acceptance still requires fresh physical input and a fully presented review.


The `session-directory` and `session-inventory` owners handle direct session
navigation independently of `global.list`. They support `next`, `previous`,
`page-up`, `page-down`, `first`, `last`, `accept`, `cancel`, and `backspace`.
Directory navigation also supports `enter-directory`: Tab browses into the
selected directory, while Enter opens it as a workspace. Inventory Tab has no
action. These owners preserve literal filter text and do not inherit leader
cancellation. Inventory cancellation returns to the session list with its
selection preserved. Directory cancellation restores the preceding editor mode.
Report hints show only the available return action.

```yaml
keys:
  ui:
    context-actions:
      next: [Ctrl-n]
      previous: [Ctrl-p]
    path:
      open-actions: [F9]
    program-actions:
      accept: [F8]
      cancel: [F9]
    path-actions:
      cancel: [F7]
```

Menu letters remain structural choices. Context menus reserve plain alphabetic
choice keys other than j/k. Path actions reserve r and s with any modifiers,
preserving their existing choice handling. An inherited leader keeps its old
cancellation precedence even if it matches a menu choice or navigation key.
Changing `cancel` restores the covered key's ordinary meaning. Hints show only
currently reachable shortcuts. Cancel in path actions returns to the path
popup. Cancel there closes it.

Prompt owners are `command-prompt` (the command palette), `text-prompt`
(search, rename, settings and other text entry), `path-prompt` (Finder path
and directory-tree Move), and `program-prompt` (Open with). They do not inherit
leader cancellation: literal characters, including spaces, remain text.

All prompt owners support `accept`, `cancel`, `save`, `move-left`, `move-right`,
`move-start`, `move-end`, `word-left`, `word-right`, `delete-backward`,
`delete-forward`, `delete-word-backward`, `delete-to-start`, `delete-to-end`,
and `backspace`. The defaults keep Ctrl-c/s/b/f/a/e, Alt-b/f, Ctrl-h/d/w/u/k,
and Backspace respectively. `backspace` resets completion selection;
`delete-backward` (Ctrl-h) preserves it. Non-command prompts additionally
support `delete` (Delete), which resets completion selection.
`delete-forward` (Ctrl-d) preserves it. The command palette's physical Delete
key remains unbound by default.

Command, path and program prompts support `next` (Down) and `previous`
(Up/Shift-Tab). Command prompts also support `first`/`last` (Home/End) and
`complete` (Tab). Path prompts support `complete` (Tab). Program prompts
support `open-actions` (Tab). Home/End in other prompts move the text cursor.
Plain characters and literal control characters cannot be assigned as prompt
actions. Defaults preserve their existing modifier rules. Configured keys
match exactly.

```yaml
keys:
  ui:
    command-prompt:
      accept: [F9]
      complete: [Ctrl-Space]
    text-prompt:
      cancel: [F10]
```

Accepting a command from an explicitly configured key keeps the same native
handoff eligibility as fresh Enter. Repeated or replayed input cannot acquire
that authority. Existing modified-Enter defaults keep their original limits.

The `file-picker` and `finder` owners support `accept`, `cancel`, `next`,
`previous`, `page-up`, `page-down`, `first`, `last`, `move-left`, `move-right`,
`move-start`, `move-end`, `backspace`, `delete-backward`, `delete`,
`delete-word-backward`, `delete-to-end`, `split-horizontal`, `split-vertical`,
and `toggle-preview`. `finder` also supports `toggle-mode` (plain Tab).
File-picker Tab aliases `next`. Modified Tab in Finder remains unbound. The
existing Ctrl-s/Ctrl-v shortcuts open files horizontally/vertically. Splits
remain unavailable for buffer and terminal rows.

These owners inherit leader cancellation only while the query is empty.
Once text exists, the leader resumes its configured ordinary meaning. Replacing
or removing `cancel` removes the inherited leader shortcut too. Literal filter
characters remain structural input. Picker `delete-backward` (Ctrl-h) keeps
its existing ranking-publication behavior. `backspace` defers held ranking
results until an action reads them.

```yaml
keys:
  ui:
    finder:
      accept: [F8]
      toggle-mode: [F9]
      toggle-preview: [F10]
    file-picker:
      split-horizontal: [Ctrl-F8]
      split-vertical: [Alt-F8]
```

Pane drag gestures can be changed independently of keyboard bindings:

```yaml
keys:
  pointer:
    pane-extend: [Ctrl-Alt-LeftDrag]
    # pane-move: [Alt-LeftDrag]     # default
    # pane-extend: [Shift-Alt-LeftDrag] # default
```

Each list replaces all gestures for that action. `[]` disables the action's
pointer bindings. Omitted actions keep their defaults, including with
`keys.defaults: false`. Modifiers match exactly. Pane gestures accept
`LeftDrag`, `MiddleDrag`, and `RightDrag`, with optional `Ctrl-`, `Alt-`,
`Shift-`, `Super-`, `Hyper-`, or `Meta-` modifiers. Shared pane borders keep
resizing priority. The action is captured when the button is pressed and lasts
through release, even when the modifiers or configuration change. Conflicting
or invalid pane bindings keep both default pane gestures and produce a
configuration diagnostic. Contextual help shows the effective gestures.
Configuration fragments merge `pointer` by action. A later action list replaces
the earlier list.

The same section exposes other editor-owned pointer actions:

| Actions | Accepted gestures | Default behavior |
| --- | --- | --- |
| `select`, `extend` | `LeftDrag`, `MiddleDrag`, `RightDrag` | Left selects; Shift-left extends the existing anchor |
| `pane-resize`, `tree-resize` | Button drags | Left drag on the corresponding border |
| `open-link` | `LeftClick`, `MiddleClick`, `RightClick` | Ctrl-left in the native window; Super-left on macOS |
| `copy` | Button clicks | Right click inside the current selection copies it |
| `scroll-up`, `scroll-down`, `scroll-left`, `scroll-right` | `WheelUp`, `WheelDown`, `WheelLeft`, `WheelRight` | Matching direction, three cells per host event |
| `tree-select` | Button clicks | Left click selects the presented tree row |
| `tree-open` | `LeftDoubleClick`, `MiddleDoubleClick`, `RightDoubleClick` | Two left clicks on the same presented row open it |
| `session-activate` | Button clicks | Left click activates the presented session-strip entry |
| `media-pan`, `media-region`, `media-auto-select` | Button drags | Middle/right or Alt-left pans; Shift-left selects a region; ordinary left selects PDF text, a wordless PDF region, or pans an image |
| `media-toggle-zoom` | Button double clicks | Ordinary left double-click toggles image fit/zoom where automatic selection would pan |
| `media-zoom`, `media-horizontal`, `media-scroll` | `Wheel` | Ctrl/Super zooms, otherwise Shift scrolls horizontally, otherwise pans or changes PDF pages at fit size |
| `preview-select` | Button drags | Left selects text in a native document preview; an inactive preview first receives focus |
| `preview-zoom`, `preview-horizontal`, `preview-scroll` | `Wheel` | Ctrl/Super zooms, otherwise Shift scrolls horizontally, otherwise ordinary two-axis scrolling |
| `input-overflow-dismiss` | Button clicks | Left click dismisses the native input-loss banner |

Drag bindings start on button-down and keep their button and target until
release. Click bindings act on button-down. Prefix any gesture with the same
exact modifiers as pane gestures. Defaults for ordinary clicks, resizing, and
wheel movement accept all modifiers. Default selection distinguishes Shift.
An authored alias always matches only its written modifiers.

```yaml
keys:
  pointer:
    select: [LeftDrag]
    extend: [Ctrl-LeftDrag]
    open-link: [Alt-MiddleClick]
    tree-open: [MiddleDoubleClick]
```

Selection and extension compete in one group, pane move and extension in
another, and the four wheel directions in a third. Replace conflicting lists
together. Inheriting a conflicting alias is an error. An invalid group keeps
its defaults without discarding valid mappings in other groups. Each action
accepts at most 192 aliases. Unknown action names reject the pointer section.

Hit regions keep their precedence. Resizing owns matching border presses,
pane gestures precede content actions, and link opening precedes live terminal
mouse reporting. Inside the active selection, copying takes priority over
selection. Elsewhere the selection gesture can still place a caret. For
unconditional right-button selection, configure `select: [RightDrag]` together
with `copy: []`. Live terminal SGR reports keep their original physical
buttons, modifiers, and wheel directions. Native media and preview gestures
have their own input ownership.

Native wheel bindings use continuous `Wheel` input, preserving trackpad deltas
and zoom pivots. They do not use the host's directional wheel steps. Media drag
actions compete in one group, media wheel actions in another, and preview wheel
actions in a third. For example:

```yaml
keys:
  pointer:
    media-pan: [MiddleDrag]
    media-region: [RightDrag]
    media-auto-select: [LeftDrag]
    media-zoom: [Alt-Wheel]
    media-horizontal: [Shift-Wheel]
    media-scroll: [Wheel]
    preview-select: [MiddleDrag]
```

Actions in one group may not share a gesture. When they do, Runyte reports the
overlap and keeps the whole group's defaults. If a list claims a gesture that
another action in the group holds by default, it must reassign that action too.
Ctrl- and Super-wheel belong to `preview-zoom` by default. An older
`preview-scroll` or `preview-horizontal` list that names them now needs its own
`preview-zoom` list.

Native surfaces consume removed gestures rather than passing them to hidden
source text. The factory preview right-click keeps source-selection copying;
replacing or removing `preview-select` also suppresses that fallback. A captured drag stays with its pane, page and document source.
Replacement or dismissal cancels the action while consuming its trailing release.
Pane gestures still take priority. Remapped native drags follow their initiating
button. Effective factory gestures keep their existing multi-button behavior.

Old-frame presses and wheel events are rejected after configuration reload.
A drag already in progress keeps its captured action. Contextual help displays
the effective pointer bindings, including removed actions as `unbound`.

`Ctrl-Alt-LeftDrag` avoids terminals that reserve Shift for native selection.
The terminal or window manager still needs to deliver the configured gesture.

`keys.bind` has one map per mode: `normal`, `select`, `insert`, and
`replace`. Each maps a key sequence to one of four values:

| Value | Meaning | Example |
| --- | --- | --- |
| action name | Run one action. | `X: extend-line-above` |
| list | Run several actions in order. | `F6: [select-all, yank]` |
| `{ action, arg }` | Run a command with an argument. | `F7: { action: pipe, arg: sort }` |
| `null` | Remove the binding in this mode. | `F8: null` |

The mapping form also accepts `{ action: yank }` without an argument. `arg`
must be a string; quote numeric or boolean text. Omission and an empty string
retain each command's existing argument semantics. Unknown fields, `null`
arguments and `{ name: arg }` shorthand are rejected. Replace the old
`{ command: pipe, argument: sort }` syntax with
`{ action: pipe, arg: sort }`; the old form is no longer accepted.
### Per-step count, register and extension

Action mappings accept these optional fields:

| Field | Values | Meaning |
| --- | --- | --- |
| `count` | `inherit`, `none`, or an integer from 1 to 999999 | Reuse the binding-entry count, run without a count, or use a fixed count. Requires a count-accepting action. |
| `register` | `inherit`, `none`, or a quoted printable Unicode scalar | Reuse the binding-entry register, use the unnamed register, or select a fixed register. Requires a register-using action. |
| `extend` | `true` or `false` | Extend from the existing anchor or move a caret for this motion only. Requires a supported motion and does not change mode. |

Without a field, the first eligible executed step consumes that pending prefix.
Later eligible steps run without a count or use the unnamed register. Fixed,
`none`, and `inherit` overrides leave the pending prefix available; skipped
branches consume nothing. `register-select` explicitly updates the pending
register, while `register: inherit` still refers to the binding-entry prefix.
An operand wait preserves both captured and pending values.

```yaml
keys:
  bind:
    normal:
      F6:
        - {action: move-right, count: 2, extend: true}
        - {action: yank, register: a}
        - {action: move-right, count: inherit, extend: false}
```

Without `extend`, the motion uses its normal selection policy. With the field,
that policy is overridden for this step, including in a preset without Select.
The action browser and generated reference list count, register, and extension
support; describe-key and copy-key-binding retain the authored options.

### Mode entry and conditional programs

`mode-enter` preserves the selection and changes the active named mode. Its
`arg` names a declared editor mode or one of the buffer type's `normal`,
`select`, `insert`, and `replace` roles. A missing role is a configuration error.

```yaml
modes:
  composing: {inherits: normal}
buffer-types:
  text: {normal: composing, select: select, insert: insert, replace: replace}
keys:
  bind:
    composing:
      F6:
        - {action: mode-enter, arg: insert}
        - insert-newline
      F7:
        if: {mode: select}
        then: [yank]
        else: [select-all, yank]
```

Both `then` and `else` must be lists; either may be empty. A mode condition
using a role name tests editing-role membership, including inherited submodes.
Other names test the exact active named mode. Other supported conditions are
`recording-macro`, `tree-entry-expanded`, `tree-entry-is-directory`, and
`tree-entry-is-root`; a tree condition must only be reached in the directory tree.
Programs contain at most 16 steps in total and four levels of conditional
nesting. A program must contain an action, even when some branches are empty.

`{action: mode-enter, arg: select, temporary: true}` saves the current mode as
the return mode for a temporary selection gesture. Ordinary non-selection
actions restore that mode before running; selection operations can continue
or consume the gesture. A later explicit `mode-enter` without `temporary: true`
clears the return mode. Changing pane focus, buffer, terminal session, or view
type ends the gesture; its return mode never follows you into another view.
A condition at the beginning of the next key program
restores the previous gesture first; a condition later in a program observes
the mode reached by preceding steps.

Configuration checks every reachable step after inheritance and removals have
been applied. Each branch contributes its possible resulting modes to the
tail. Selection undo/redo and syntax shrink can restore historical modes;
insert an explicit `mode-enter` before a tail that needs one particular mode.
Diagnostics identify the buffer type, named mode, key and failing step. A bad
binding is rejected independently. Invalid graph changes restore the preset
graph while retaining valid key edits. Read-only buffers, terminal state and
unavailable language services remain runtime refusals and stop the program.

Action names are the hyphenated names in the
[reference](custom-key-bindings.md#bindable-action-reference). An action does not need a default key
to be bindable. Any action or colon command can also be named the way the
command palette names it, such as `theme` for `open-theme-settings` or `help`
for `show-help`.

Not everything is bindable: internal transitions, retired grammar commands,
unsupported actions, and list-only actions are excluded.

A complete example:

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
      F7: { action: pipe, arg: sort }
      F8: null
    select:
      X: extend-line-above
    insert:
      Ctrl-s: save
```

**Modes are independent.** A binding in `normal` does nothing in `select`, and
`insert` and `replace` do not inherit from each other. Repeat a binding in
each mode where you want it.

**Arguments** use the command palette's own parsing rules.

- A structured value can also appear inside a list.
- Commands with a required argument must be given one.
- Actions do not gain new arguments by being bindable.

### Key binding recipes

**Yank the whole buffer:**

```yaml
keys:
  bind:
    normal:
      F6: [select-all, yank]
```

`yank` leaves the selection and mode alone, so the buffer stays selected
afterwards. The default `y` program adds `collapse-selection` and `mode-enter
normal`; terminal `y` omits the collapse. The former bundled
`yank-collapse-normal-mode` and `yank-line-collapse-normal-mode` actions now
report their replacement programs.

**Sort the selected lines:**

```yaml
keys:
  bind:
    normal:
      Alt-s: { action: pipe, arg: sort }
    select:
      Alt-s: { action: pipe, arg: sort }
```

**Switch to a particular theme** (`theme` is the palette name of
`open-theme-settings`):

```yaml
keys:
  bind:
    normal:
      F10: { action: theme, arg: mocha }
```

**Open a file:**

```yaml
keys:
  bind:
    normal:
      F11: { action: open, arg: README.md }
```

**Select a line and start typing at its end:**

```yaml
keys:
  bind:
    normal:
      Alt-a: [select-line, insert-line-end]
```

**Stop a key from doing anything** (here `J`, which joins lines by default).
Remove it in each mode where it is bound:

```yaml
keys:
  bind:
    normal:
      J: null
    select:
      J: null
```

**Open the action list with `F2`** (`help` is the palette name of
`show-help`):

```yaml
keys:
  bind:
    normal:
      F2: { action: help, arg: key-actions }
```

### Without the built-in keys

Everything above changes Runyte's own keyboard: keys you do not mention keep
their built-in meaning. Set `keys.defaults: false` to start from an empty one
instead, so the only keys that do anything are the ones `bind` assigns:

```yaml
keys:
  defaults: false
  bind:
    normal:
      ":": open-command-palette
      i: enter-insert-mode
      h: move-left
      l: move-right
    select:
      Esc: enter-normal-mode
    insert:
      Esc: enter-normal-mode
      Enter: insert-newline
      Backspace: delete-char-backward
    replace:
      Esc: enter-normal-mode
```

- **What goes:** every built-in key in Normal, Select, Insert, and Replace
  modes that `bind` could assign. That includes the whole leader and window
  menus. It also includes keys that only show a message about a command Runyte
  does differently or doesn't have yet. That includes Insert mode's editing
  keys. Printable characters still type, but `Enter`, `Backspace`, `Delete`,
  `Tab`, the arrows, `Home`, `End`, and the page keys do nothing until `bind`
  assigns them.
- **What stays:** keys that are not yours to assign. Special buffers such as
  the explorer, Git views, and help keep their own keys. So do lists, prompts,
  and terminals. Terminals keep answering to the built-in Insert keys, such as
  `Ctrl-C` to copy and the window menu, as they do whenever `bind` replaces
  one. `Tab`, which runs a code action, and the count digits stay too, because
  `bind` cannot use them.
- **Two exits stay too, unless `bind` reassigns them.** `Ctrl-\` (`Ctrl-4` on
  legacy terminals) still returns to Normal mode. Inside a terminal, `Esc`
  belongs to the program, and `Ctrl-\` is the only way out of its input. With
  `editor.fast_pane_keys` on, `Ctrl-h`, `Ctrl-j`, `Ctrl-k`, and `Ctrl-l` keep
  moving between panes.
- **`rebind` is ignored**, with a message for each entry, since there is no
  built-in binding left to move. `leader` and `window` still set what `Leader`
  and `Window` mean in `bind`, and may be keys that only a built-in binding
  used.
- **A way out is required.** The keyboard must reach `open-command-palette` in
  Normal mode and `enter-normal-mode` in Select, Insert, and Replace modes,
  directly or as one of a sequence of actions. `Ctrl-\` covers the second
  unless you reassign both of its spellings. Without them you couldn't leave a
  mode or reach the configuration to fix it. In that case Runyte reports what
  is missing and keeps the built-in keys underneath your `bind`.

`:write-config` keeps `defaults: false` and writes only your own keys, without
the commented built-in examples, since none are underneath.

### How bindings are resolved

- **After remapping.** `keys.bind` sees the keyboard as `keys.rebind` and the
  prefixes left it. Left sides are final key sequences, and `Leader` and
  `Window` expand to the configured prefixes.
- **Exact replacement.** Assigning a key replaces whatever global action it
  had in that mode.
- **No actions on prefixes.** Assigning an action to a prefix that still has
  keys under it is an error.
- **Reserved keys.** In Normal and Select, a leading `1`–`9` is still a count
  and `Tab` still opens context actions. `Escape` and `Backspace` cannot appear
  inside a sequence. Named prefixes and `keys.rebind` must respect these
  reservations too. Unreachable remappings are rejected and keep their
  working defaults.
- **Scoped input wins.** Buffer-specific keys, prompts, lists, native
  confirmations, and terminal input keep handling their own keys.

### Action sequences

A list runs its actions directly, one after another. It never types keys or
triggers other bindings.

**Selection and mode steps.** `select-all`, `collapse-selection`, `toggle-case`
and `replace-char` preserve the current mode. Their default keys (`%`, `;`,
`~` and `r`) include a separate internal mode step, so the keys behave as
before. `fit-selection-for-normal-mode` fits the selection without changing
mode: it collapses ranges and applies the existing Insert-exit caret rules.
Escape runs that action followed by a Normal `mode-enter` step. Public
mode-entry actions retain their selection-fitting behavior, except
`enter-select-mode`, which now only enters Select. The default `v` key toggles
through a conditional: from Select it collapses and enters Normal;
otherwise it enters Select. Half-open mouse and syntax selections convert to
inclusive endpoints before collapse, so the resulting caret may be one
character before the old endpoint. On native media, leaving Select retains the
media selection for copying and does not move the hidden document caret.

The default macro-recording key also uses a conditional.
`macro-record-default` starts recording and refuses an existing recording;
`Space m m` stops any active recording, or starts the default one.
Directory-tree `h`/Left, `l`/Right, Enter, `v` and `s` use tree conditions with
`directory-tree-collapse`, `directory-tree-select-parent`,
`directory-tree-expand`, `directory-tree-select-first-child` and file-opening
actions. The old `directory-tree-left` and `directory-tree-right` names report
migration advice. Directory file-opening actions no longer toggle directories.

Conditional and pure mode steps use the public syntax described above and can
be copied as configuration. `defaults: false` removes their default keys.

**Supplied operands and prompts.** `arg` is a string, with a 4096-byte limit.
The supported key-program arguments are:

| Actions | Argument |
| --- | --- |
| `find-next-char`, `find-previous-char`, `find-till-next-char`, `find-till-previous-char`, `replace-char`, `surround-add`, `surround-delete`, `register-select`, `macro-record`, `macro-replay` | Exactly one Unicode scalar, not a key spelling |
| `surround-replace` | Exactly two Unicode scalars: the old delimiter, then the new delimiter |
| `search`, `search-regex` | Nonempty document search pattern; a successful search enters Select |
| `terminal-search`, `terminal-search-regex` | Nonempty terminal review pattern; preserves the mode and document query |
| `directory-tree-search` | Nonempty regular expression over visible tree entries |
| `keep-matching-selections`, `remove-matching-selections` | Nonempty regular expression |
| `join-selections` | Literal delimiter; an empty string joins directly |
| `insert-text` | Nonempty literal text at every caret, including newlines and tabs |
| `search-workspace`, `search-workspace-regex` | Nonempty pattern; remains last-only because the search runs in the background |

`insert-text` preserves the document's selected text: it inserts at each range's
head, then leaves a caret after the insertion. It works in every editing role
without changing modes, automatic indentation or pairing. Use an explicit
`delete-selection` or `delete-selection-for-change` step first for replacement.
All insertions form one transaction under the program's undo checkpoint;
read-only buffers refuse the edit. Empty strings are rejected at load.

Existing palette arguments remain available. Omitting an optional argument keeps
the interactive operand or prompt. Supplied arguments do not answer confirmations
or multi-step prompts, and do not resume a program after an interactive prompt.
An invalid pattern or a search with no matches stops the remaining steps.

```yaml
keys:
  bind:
    normal:
      F6: [{action: find-next-char, arg: ":"}, move-right]
      F7: [{action: search-regex, arg: "TODO|FIXME"}, yank]
      F8: [{action: surround-replace, arg: '(]'}, yank]
```

**Order.** Continuation follows each action's effects, including effects in
specialized buffer types. Selection, text, mode, view, pane and other synchronous
effects can continue. A character operand pauses the list; after it arrives,
the next step runs. Escape cancels the remaining steps.

Prompt and background effects must end the list. A supplied argument that
replaces a prompt removes that prompt requirement, but does not remove background
work. For example, `{ action: terminal-rename, arg: build }` can continue;
`file-picker-path` still starts a scan and `pipe` still starts a worker, even
with arguments. Plugin commands and queued macro replay must be last. The `goto-word` label
chooser also owns input and remains last-only.

Previously rejected lists such as `[enter-insert-mode, move-right,
enter-normal-mode]`, `[find-next-char, yank]`, `[surround-replace, yank]`,
`[focus-window-right, move-left]`, and `[show-help, move-file-end]` are now
accepted. Colon commands with only synchronous effects can also continue.
Mixed-effect actions such as `split-vertical` and `split-horizontal` remain
last-only because their path-taking forms can prompt or start background work.
Each step must be admitted in every mode that can reach it. Use explicit
`mode-enter` steps to establish the mode required by later actions.

Nothing resumes a list after a prompt, confirmation, or background operation.
If an earlier step opens a list or popup, a subsequent character-operand action
is unavailable while that surface owns input; the program stops without leaving
a pending operand behind.

**Failure.** An error or unavailable action stops the list. So does
`search-next` or `search-previous` finding no match, so the actions after it
never run against a selection the search did not choose. Mode-preserving word,
delimiter, syntax-object and syntax-relation actions likewise stop any configured
list when they produce no selection. For example, `[select-inside-parentheses,
move-right]` leaves the cursor in place when no enclosing pair exists.

- Actions that already ran stay applied.
- A list is not rolled back. A saved file stays saved.

**Counts.** Every binding follows the same rule: a count goes to the first
action whose descriptor accepts one; later actions run once. If no action
accepts a count, the count is dropped silently and the binding runs. This
applies to built-in and configured bindings, including single actions,
parameterized actions, colon commands and plugin commands. For example, `3`
before `[yank, move-right, move-right]` yanks once and moves right four times.
During macro replay, an accepted repetition count in a multi-action binding
is subject to the existing 128-repetition atomic limit; exceeding that limit
stops replay before any step runs. Dropped counts consume no repetition work.
Singleton counted commands retain their cooperative replay scheduling.

Paste actions interpret a count as N copies in one transaction, deciding once
whether each range is replaced or receives an insertion. The existing
`p`, `P`, `Space c p` and `Space c P` actions still enter Normal after
successful document paste. A counted paste selects the entire replacement.
For example, Select mode on `a` in `abc`, register `x`, and `3p` produces
`xxxbc` with `xxx` selected. Earlier versions left a caret after repeated
invocations; longer registers could repeatedly replace the same range instead
of inserting all copies. Expanded paste payloads are limited to 64 MiB across
all selections (1 MiB for terminal input).

Removed action names report their replacements: `yank-delete-selection`
and `yank-change-selection` become
`cut-selection` and `cut-selection-for-change` followed by Normal or Insert.
`change-selection` becomes `delete-selection-for-change` followed by Insert.
Both no-yank delete actions preserve mode: `delete-selection` clamps the caret
and honors transient whole rows; `delete-selection-for-change` preserves the
insertion seam and deletes operative spans. Neither writes a register.
`set-search-from-selection` is now `search-set-from-selection`.
`search-selection` becomes query setup, `search-select-all-matches`, and pure
Select entry. These exact default programs require internal mode steps until
the public binding format supports pure mode entry. The additive
`search-extend-next`/`search-extend-previous` actions preserve mode;
`search-next` and `search-previous` still enter Select in documents. Extension retains existing ranges and makes the added
match primary, wrapping through the query's matches.

Word and delimiter objects, syntax objects, expansion and relation selections, flipping a
selection, and splitting at line edges preserve the current mode. Their default
keys explicitly enter Select (Normal for line-edge splits). If a selection
command finds no result, its program stops with an informational status; a
counted command that selected successfully before reaching a boundary still
continues to its mode step. These actions retire transient `x`/`X` mode restoration.
Syntax shrink restores the selection, coordinate semantics and mode saved by
expansion. Each expansion records the mode in which it runs; explicit mode
steps do not invalidate that history. In Normal, `2 Space x e` records Normal
for both repetitions before the explicit Select step; the first shrink therefore
returns to Normal. The old bundled repeat recorded Select in its later frames.
Two separate `Space x e` presses still shrink through Select back to Normal.
Paragraph objects retain whole-row
selection and mode behavior until the shared line-selection split.

**Undo.**

- A list of selection-only actions is one selection-history step.
- A binding's text edits form one undo step in each buffer it edits, including
  edits separated by character operands. Cancellation or failure keeps edits
  already made together; later unrelated edits start a separate step.
- A program ending in Insert or Replace keeps its group open for typing until
  that mode is left. For example, `[delete-selection, enter-insert-mode]` and
  the text typed afterward undo together.
- Explicit `undo`, `redo` and `commit-undo-checkpoint` actions retain their
  history boundaries. A save's whitespace cleanup belongs to the program.
  Separate bindings replayed by a macro
  remain separate steps, while a counted binding remains one step.

Paste stays literal, and macros record the physical keys you press, as before.

### Limits and errors

Mode-entry role targets are checked against every buffer type that can reach
the binding. A missing role reports the buffer type, mode, key and role, and
restores the preset role graph. Remove or replace inherited bindings that enter
a role before omitting that role. Effective overrides and exact removals apply
before this check; both conditional branches are checked.

| Limit | Value |
| --- | --- |
| Assignments in `keys.bind`, across all files | 65,536 |
| Keys per sequence | 8 |
| Actions per binding | 16 |
| Argument length | 4,096 bytes, no control characters |

- An empty list is rejected. Use `null` to remove a binding.
- Problems in `keys` never stop Runyte from starting. Malformed sections,
  unknown fields, bad key spellings, unmatched defaults, overlong sequences,
  and conflicts are reported in a `Key bindings` error notification. The
  affected section or rules are dropped and the built-in behavior stays.
- One bad assignment does not cancel valid remappings. Conflicting
  assignments are rejected whatever their order in the file.
- YAML syntax errors, and invalid settings outside `keys`, still stop startup
  as usual.

**Help and hints follow your bindings.** Help lists complete actions and
arguments. Key hints show the same descriptions, shortened to fit. A key you
removed or overwrote is no longer advertised. Teaching text shows another
binding for the action, or says it is unbound.

### Discovering actions and bindings

`Space h` is the Help prefix: `Space h k` runs `:describe-key`, and `Space h a`
runs `:browse-actions`. `Space ?` still opens contextual help. The companion
`:copy-key-binding` command has no default key.
Describe key captures a sequence in the current view and mode without running
it. Escape cancels. The result reports the accepted configuration source and
line when available, action steps, and YAML that the current compiler accepts.
Runtime plugin bindings use their plugin configuration instead. The copy command
copies the most recently inspected copyable binding.

The action browser opens a searchable picker. Enter inspects an action; Tab
filters to the current view and mode. Details distinguish plain effective keys
from membership in composed bindings, and show descriptor contracts and runtime
capability gates. Document state and action-specific guards can still prevent an
action from running.

Whole-keymap export (`:keymap-export`, without a default key) is planned for
stage 4, alongside the merged binding table. It is not available yet.

### Bindable action reference

Regenerate this block with `cargo run --example action_reference`.

<a id="mode-and-page-actions"></a>
<a id="insert-and-replace-mode-actions"></a>
<a id="text-object-actions"></a>
<a id="syntax-tree-actions"></a>
<a id="file-buffer-and-destination-actions"></a>
<a id="pane-actions"></a>
<a id="persistent-session-actions"></a>
<a id="language-server-actions"></a>
<a id="clipboard-register-and-macro-actions"></a>
<!-- action-reference:start -->

Use these names in `keys.bind`. Modes are independent. Sequence position says whether
an action may be followed by another action. Counts go to the first accepting step in every binding. If none accepts a count,
the count is dropped silently and the binding runs. Plugin bindings use `plugins[].bindings`.

#### Application actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `mode-enter` | normal, select, insert, replace | can be followed by another action | Enter a mode, preserving the selection |
| `enter-normal-mode` | normal, select, insert, replace | can be followed by another action | Fit selection and enter Normal mode |
| `open-command-palette` | normal, select | must be the last action | Open the command palette |

#### Editing actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `enter-insert-mode` | normal, select | can be followed by another action | Insert before the selection |
| `enter-replace-mode` | normal, select | can be followed by another action | Replace text from each selection head |
| `append-after` | normal, select | can be followed by another action | Insert after the selection |
| `insert-line-start` | normal, select | can be followed by another action | Insert at line start |
| `insert-line-end` | normal, select | can be followed by another action | Insert at line end |
| `open-line-below` | normal, select | can be followed by another action | Open a line below |
| `open-line-above` | normal, select | can be followed by another action | Open a line above |
| `replace-char` | normal, select | can be followed by another action | Replace selection with a character |
| `surround-add` | normal, select | can be followed by another action | Surround selections with a pair |
| `surround-replace` | normal, select | can be followed by another action | Replace the surrounding pair |
| `surround-delete` | normal, select | can be followed by another action | Delete the surrounding pair |
| `toggle-case` | normal, select | can be followed by another action | Switch case of the selection |
| `undo` | normal, select | can be followed by another action | Undo the last change |
| `redo` | normal, select | can be followed by another action | Redo the last change |
| `yank` | normal, select | can be followed by another action | Yank the selection or character |
| `yank-line` | normal, select | can be followed by another action | Yank the lines the selection touches |
| `paste-after-or-replace` | normal, select | can be followed by another action | Replace the selection, or paste after the caret |
| `paste-after` | normal, select | can be followed by another action | Paste after the selection or caret |
| `replace-with-register` | normal, select | can be followed by another action | Replace the selection or character |
| `paste-before` | normal, select | can be followed by another action | Paste before the selection |
| `indent` | normal, select | can be followed by another action | Indent the selected lines |
| `unindent` | normal, select | can be followed by another action | Unindent the selected lines |
| `toggle-comments` | normal, select, insert, replace | can be followed by another action | Comment or uncomment the selected lines |
| `cut-selection` | normal, select | can be followed by another action | Yank, then delete the selection or character |
| `cut-selection-for-change` | normal, select | can be followed by another action | Yank, then change the selection or character |
| `delete-selection` | normal, select | can be followed by another action | Delete the selection or character without yanking |
| `delete-selection-for-change` | normal, select | can be followed by another action | Delete without yanking, preserving the insertion seam and mode |
| `trim-trailing-whitespace` | normal, select | can be followed by another action | Delete trailing whitespace from every selected line |
| `hard-wrap` | normal, select | can be followed by another action | Hard-wrap the selection |
| `reflow` | normal, select | can be followed by another action | Reflow paragraphs in the selection |
| `insert-text` | normal, select, insert, replace | can be followed by another action | Insert literal text at each caret |
| `join-selections` | normal, select | must be the last action | Join the selected lines with a typed delimiter |
| `join-lines` | normal, select | can be followed by another action | Join the selected lines, or the line below, with a space |
| `format-table` | normal, select | can be followed by another action | Align the columns of the selected table |
| `delete-word-backward` | insert, replace | can be followed by another action | Delete the previous word |
| `delete-word-forward` | insert, replace | can be followed by another action | Delete the next word |
| `delete-to-line-start` | insert, replace | can be followed by another action | Delete to the start of the line |
| `delete-to-line-end` | insert, replace | can be followed by another action | Delete to the end of the line |
| `delete-char-backward` | insert, replace | can be followed by another action | Delete the previous character |
| `delete-char-forward` | insert, replace | can be followed by another action | Delete the next character |
| `insert-newline` | insert, replace | can be followed by another action | Insert a new line |
| `insert-indent` | insert, replace | can be followed by another action | Insert configured indentation |
| `insert-other-indent` | insert, replace | can be followed by another action | Insert the other indentation style |

#### Movement actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `fit-selection-for-normal-mode` | normal, select, insert, replace | can be followed by another action | Fit selection for Normal mode |
| `move-left` | normal, select, insert, replace | can be followed by another action | Move left |
| `move-right` | normal, select, insert, replace | can be followed by another action | Move right |
| `move-up` | normal, select, insert, replace | can be followed by another action | Move up |
| `move-down` | normal, select, insert, replace | can be followed by another action | Move down |
| `move-line-start` | normal, select, insert, replace | can be followed by another action | Move to line start |
| `move-line-end` | normal, select, insert, replace | can be followed by another action | Move to line end |
| `move-first-non-whitespace` | normal, select | can be followed by another action | Move to first non-whitespace character |
| `move-file-start` | normal, select | can be followed by another action | Move to file start |
| `move-file-end` | normal, select | can be followed by another action | Move to file end |
| `move-word-forward` | normal, select | can be followed by another action | Move to next word start |
| `move-word-backward` | normal, select | can be followed by another action | Move to previous word start |
| `move-word-end` | normal, select | can be followed by another action | Move to next word end |
| `move-long-word-forward` | normal, select | can be followed by another action | Move to next WORD start |
| `move-long-word-backward` | normal, select | can be followed by another action | Move to previous WORD start |
| `move-long-word-end` | normal, select | can be followed by another action | Move to next WORD end |
| `goto-next-paragraph` | normal, select | can be followed by another action | Go to the next paragraph |
| `goto-previous-paragraph` | normal, select | can be followed by another action | Go to the previous paragraph |
| `find-next-char` | normal, select | can be followed by another action | Find next character |
| `find-previous-char` | normal, select | can be followed by another action | Find previous character |
| `find-till-next-char` | normal, select | can be followed by another action | Find before next character |
| `find-till-previous-char` | normal, select | can be followed by another action | Find after previous character |
| `page-up` | normal, select, insert, replace | can be followed by another action | Move one page up |
| `page-down` | normal, select, insert, replace | can be followed by another action | Move one page down |
| `half-page-up` | normal, select | can be followed by another action | Move half a page up |
| `half-page-down` | normal, select | can be followed by another action | Move half a page down |
| `goto-window-top` | normal, select | can be followed by another action | Move to the top of the view |
| `goto-window-center` | normal, select | can be followed by another action | Move to the center of the view |
| `goto-window-bottom` | normal, select | can be followed by another action | Move to the bottom of the view |
| `goto-file` | normal, select | must be the last action | Open the file or web link under the cursor |
| `goto-word` | normal, select | must be the last action | Label visible words by proximity and jump to one |
| `jump-backward` | normal, select | can be followed by another action | Jump to the previous position |
| `jump-forward` | normal, select | can be followed by another action | Jump to the next position |
| `jump-backward-buffer` | normal, select | can be followed by another action | Jump to the previous buffer or terminal surface |
| `jump-forward-buffer` | normal, select | can be followed by another action | Jump to the next buffer or terminal surface |

#### Selection actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `selection-undo` | normal, select | can be followed by another action | Undo the last selection change |
| `selection-redo` | normal, select | can be followed by another action | Redo the last selection change |
| `enter-select-mode` | normal, select | can be followed by another action | Enter Select mode |
| `select-line` | normal, select | can be followed by another action | Select the current line, then extend downward |
| `extend-line-above` | normal, select | can be followed by another action | Extend whole lines above |
| `extend-line-below` | normal, select | can be followed by another action | Extend whole lines below |
| `select-line-up` | normal, select | can be followed by another action | Select the current line, then extend upward |
| `select-all` | normal, select | can be followed by another action | Select all text |
| `collapse-selection` | normal, select | can be followed by another action | Collapse selection to the cursor |
| `flip-selection` | normal, select | can be followed by another action | Flip the selection anchor and cursor |
| `select-around-parentheses` | normal, select | can be followed by another action | Select around parentheses |
| `select-inside-parentheses` | normal, select | can be followed by another action | Select inside parentheses |
| `select-around-square-brackets` | normal, select | can be followed by another action | Select around square brackets |
| `select-inside-square-brackets` | normal, select | can be followed by another action | Select inside square brackets |
| `select-around-braces` | normal, select | can be followed by another action | Select around braces |
| `select-inside-braces` | normal, select | can be followed by another action | Select inside braces |
| `select-around-angle-brackets` | normal, select | can be followed by another action | Select around angle brackets |
| `select-inside-angle-brackets` | normal, select | can be followed by another action | Select inside angle brackets |
| `select-around-double-quotes` | normal, select | can be followed by another action | Select around double quotes |
| `select-inside-double-quotes` | normal, select | can be followed by another action | Select inside double quotes |
| `select-around-single-quotes` | normal, select | can be followed by another action | Select around single quotes |
| `select-inside-single-quotes` | normal, select | can be followed by another action | Select inside single quotes |
| `select-around-backticks` | normal, select | can be followed by another action | Select around backticks |
| `select-inside-backticks` | normal, select | can be followed by another action | Select inside backticks |
| `select-inside-word` | normal, select | can be followed by another action | Select the word under the cursor |
| `select-around-word` | normal, select | can be followed by another action | Select the word under the cursor and the space beside it |
| `select-inside-long-word` | normal, select | can be followed by another action | Select the WORD under the cursor |
| `select-around-long-word` | normal, select | can be followed by another action | Select the WORD under the cursor and the space beside it |
| `select-inside-paragraph` | normal, select | can be followed by another action | Select the paragraph's lines |
| `select-around-paragraph` | normal, select | can be followed by another action | Select the paragraph's lines and the blank lines beside them |
| `select-around-closest-delimiter` | normal, select | can be followed by another action | Select around the closest delimiter pair |
| `select-inside-closest-delimiter` | normal, select | can be followed by another action | Select inside the closest delimiter pair |
| `split-selection-at-line-ends` | normal, select | can be followed by another action | Place a cursor at the end of every selected line |
| `split-selection-at-line-starts` | normal, select | can be followed by another action | Place a cursor at the start of every selected line |
| `keep-primary-selection` | normal, select | can be followed by another action | Drop every selection except the primary |
| `remove-primary-selection` | normal, select | can be followed by another action | Drop the primary selection |
| `copy-selection-down` | normal, select | can be followed by another action | Add a cursor on the line below |
| `copy-selection-up` | normal, select | can be followed by another action | Add a cursor on the line above |
| `copy-selection-down-padded` | normal, select | can be followed by another action | Add a cursor on the line below, padding it when needed |
| `copy-selection-up-padded` | normal, select | can be followed by another action | Add a cursor on the line above, padding it when needed |
| `rotate-selection-forward` | normal, select | can be followed by another action | Make the next selection primary |
| `rotate-selection-backward` | normal, select | can be followed by another action | Make the previous selection primary |
| `rotate-selection-contents-forward` | normal, select | can be followed by another action | Rotate selected text forward |
| `rotate-selection-contents-backward` | normal, select | can be followed by another action | Rotate selected text backward |
| `keep-matching-selections` | normal, select | must be the last action | Keep selections matching a regular expression |
| `remove-matching-selections` | normal, select | must be the last action | Remove selections matching a regular expression |
| `align-selections` | normal, select | can be followed by another action | Pad with spaces so every cursor shares the rightmost column |
| `trim-selections` | normal, select | can be followed by another action | Trim whitespace from every selection |

#### Syntax actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `syntax-expand-selection` | normal, select | can be followed by another action | Expand to the enclosing syntax node |
| `syntax-shrink-selection` | normal, select | can be followed by another action | Shrink the syntax selection |
| `select-syntax-parent` | normal, select | can be followed by another action | Select the enclosing syntax node |
| `select-syntax-child` | normal, select | can be followed by another action | Select the first child syntax node |
| `select-previous-syntax-sibling` | normal, select | can be followed by another action | Select the previous sibling syntax node |
| `select-next-syntax-sibling` | normal, select | can be followed by another action | Select the next sibling syntax node |
| `select-syntax-function` | normal, select | can be followed by another action | Select the enclosing function |
| `select-inside-syntax-function` | normal, select | can be followed by another action | Select inside the enclosing function |
| `select-syntax-class` | normal, select | can be followed by another action | Select the enclosing class-like item |
| `select-inside-syntax-class` | normal, select | can be followed by another action | Select inside the enclosing class-like item |
| `select-syntax-parameter` | normal, select | can be followed by another action | Select the enclosing parameter |
| `select-inside-syntax-parameter` | normal, select | can be followed by another action | Select inside the enclosing parameter |
| `goto-previous-syntax-function` | normal, select | can be followed by another action | Go to the previous function |
| `goto-next-syntax-function` | normal, select | can be followed by another action | Go to the next function |
| `goto-previous-syntax-class` | normal, select | can be followed by another action | Go to the previous class-like item |
| `goto-next-syntax-class` | normal, select | can be followed by another action | Go to the next class-like item |
| `goto-previous-syntax-parameter` | normal, select | can be followed by another action | Go to the previous parameter |
| `goto-next-syntax-parameter` | normal, select | can be followed by another action | Go to the next parameter |
| `syntax-document-outline` | normal, select | can be followed by another action | Open the document outline |
| `syntax-toggle-fold` | normal, select | can be followed by another action | Toggle the syntax fold at the cursor |
| `syntax-fold-all` | normal, select | can be followed by another action | Fold every syntax region |
| `syntax-unfold-all` | normal, select | can be followed by another action | Unfold every syntax region |

#### Search actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `search` | normal, select | must be the last action | Search for text, ignoring case |
| `search-regex` | normal, select | must be the last action | Search with a regular expression |
| `search-next` | normal, select | can be followed by another action | Select only the next search match |
| `search-previous` | normal, select | can be followed by another action | Select only the previous search match |
| `search-extend-next` | normal, select | can be followed by another action | Add the next search match as the primary selection |
| `search-extend-previous` | normal, select | can be followed by another action | Add the previous search match as the primary selection |
| `search-select-all-matches` | normal, select | can be followed by another action | Select every match of the current search |
| `search-set-from-selection` | normal, select | can be followed by another action | Use the selection or word as the search |
| `search-workspace` | normal, select | must be the last action | Search the workspace, ignoring case |
| `search-workspace-regex` | normal, select | must be the last action | Search the workspace with a regular expression |

#### View actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `align-view-center` | normal, select | can be followed by another action | Center the cursor line in the view |
| `align-view-top` | normal, select | can be followed by another action | Align the cursor line at the top |
| `align-view-bottom` | normal, select | can be followed by another action | Align the cursor line at the bottom |
| `align-view-middle` | normal, select | can be followed by another action | Center the cursor column in the view |
| `scroll-view-down` | normal, select | can be followed by another action | Scroll the view down |
| `scroll-view-up` | normal, select | can be followed by another action | Scroll the view up |
| `toggle-soft-wrap` | normal, select | can be followed by another action | Toggle soft wrapping |
| `toggle-whitespace` | normal, select | can be followed by another action | Toggle whitespace markers |
| `toggle-zen` | normal, select | can be followed by another action | Toggle the centred, maximized writing view |
| `toggle-fullscreen` | normal, select | can be followed by another action | Toggle the active pane across the whole editor area |

#### File actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `directory-open` | normal, select | must be the last action | Open file explorer in the active buffer's directory |
| `directory-tree-toggle` | normal, select | can be followed by another action | Show or hide the directory tree |
| `directory-tree-focus` | normal, select | can be followed by another action | Reveal the active file in the directory tree |
| `directory-open-working-directory` | normal, select | must be the last action | Open file explorer in the working directory |
| `open-file-picker` | normal, select | must be the last action | Open the finder over the project's files, buffers, and terminals |
| `open-all-files-picker` | normal, select | must be the last action | Open the finder over the project, including files Git ignores |
| `open-path-file-picker` | normal, select | must be the last action | Open the finder in a chosen path, including files Git ignores |
| `save` | normal, select, insert, replace | must be the last action | Write the active buffer |
| `new-buffer` | normal, select | can be followed by another action | Open a new scratch buffer |
| `open-navigator` | normal, select, insert, replace | can be followed by another action | Navigate open buffers and terminals |
| `previous-destination` | normal, select, insert, replace | can be followed by another action | Return to the previous destination in this pane |
| `session-previous` | normal, select, insert, replace | must be the last action | Return to the previously visited persistent session |
| `session-1` | normal, select | must be the last action | Attach to persistent session 1 |
| `session-2` | normal, select | must be the last action | Attach to persistent session 2 |
| `session-3` | normal, select | must be the last action | Attach to persistent session 3 |
| `session-4` | normal, select | must be the last action | Attach to persistent session 4 |
| `session-5` | normal, select | must be the last action | Attach to persistent session 5 |
| `session-6` | normal, select | must be the last action | Attach to persistent session 6 |
| `session-7` | normal, select | must be the last action | Attach to persistent session 7 |
| `session-8` | normal, select | must be the last action | Attach to persistent session 8 |
| `session-9` | normal, select | must be the last action | Attach to persistent session 9 |
| `session-next-running` | normal, select, insert, replace | must be the last action | Visit the next running persistent session |
| `session-previous-running` | normal, select, insert, replace | must be the last action | Visit the previous running persistent session |
| `open-buffer-picker` | normal, select | can be followed by another action | Open the buffer picker |

#### Git actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `git-conflicts` | normal, select | must be the last action | Open unresolved Git conflicts |

#### Window actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `split-vertical` | normal, select | must be the last action | Create a side-by-side split |
| `split-horizontal` | normal, select | must be the last action | Create a stacked split |
| `focus-window-left` | normal, select, insert, replace | can be followed by another action | Focus the pane to the left |
| `focus-window-down` | normal, select, insert, replace | can be followed by another action | Focus the pane below |
| `focus-window-up` | normal, select, insert, replace | can be followed by another action | Focus the pane above |
| `focus-window-right` | normal, select, insert, replace | can be followed by another action | Focus the pane to the right |
| `next-window` | normal, select, insert, replace | can be followed by another action | Focus the next pane |
| `swap-window` | normal, select, insert, replace | can be followed by another action | Swap this pane's content with the previously focused pane |
| `move-window-left` | normal, select, insert, replace | can be followed by another action | Move pane to the left edge, full height |
| `move-window-down` | normal, select, insert, replace | can be followed by another action | Move pane to the bottom edge, full width |
| `move-window-up` | normal, select, insert, replace | can be followed by another action | Move pane to the top edge, full width |
| `move-window-right` | normal, select, insert, replace | can be followed by another action | Move pane to the right edge, full height |
| `close-window` | normal, select | can be followed by another action | Close the active pane |
| `only-window` | normal, select | can be followed by another action | Close every pane except the active pane |
| `equalize-windows` | normal, select | can be followed by another action | Equalize pane widths, then pane heights within each column |

#### Language actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `goto-definition` | normal, select | must be the last action | Go to definition |
| `goto-declaration` | normal, select | must be the last action | Go to declaration |
| `goto-type-definition` | normal, select | must be the last action | Go to type definition |
| `goto-references` | normal, select | must be the last action | Go to references |
| `goto-implementation` | normal, select | must be the last action | Go to implementation |
| `show-documentation` | normal, select | must be the last action | Show documentation |
| `document-symbols` | normal, select | must be the last action | Open document symbols |
| `workspace-symbols` | normal, select | must be the last action | Open workspace symbols |
| `diagnostics` | normal, select | can be followed by another action | Open diagnostics |
| `trigger-completion` | normal, select, insert, replace | must be the last action | Ask the language server for completions |
| `rename-symbol` | normal, select | must be the last action | Rename symbol |
| `code-action` | normal, select | must be the last action | Apply a code action |
| `match-bracket` | normal, select | can be followed by another action | Go to the matching syntax bracket |

#### Clipboard actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `clipboard-paste-after` | normal, select | can be followed by another action | Paste from the system clipboard |
| `clipboard-paste-before` | normal, select | can be followed by another action | Paste before from the system clipboard |
| `clipboard-yank` | normal, select, insert, replace | can be followed by another action | Yank to the system clipboard |
| `clipboard-paste` | normal, select, insert, replace | can be followed by another action | Paste an image or text from the clipboard |

#### Register actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `register-select` | normal, select | can be followed by another action | Select a register |
| `macro-record` | normal, select | can be followed by another action | Record a macro named by the next key |
| `macro-record-default` | normal, select | can be followed by another action | Start recording the default macro |
| `macro-stop-recording` | normal, select | can be followed by another action | Stop recording the current macro |
| `macro-replay` | normal, select | must be the last action | Replay the macro named by the next key |
| `macro-replay-default` | normal, select | must be the last action | Replay the default macro |
| `macro-list` | normal, select | can be followed by another action | List the recorded macros |

#### Configuration actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `open-settings` | normal, select | can be followed by another action | Open editor settings |
| `open-theme-settings` | normal, select | can be followed by another action | Choose and save the editor theme |

#### Terminal actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `terminal-open` | normal, select | must be the last action | Start a terminal in this pane |
| `terminal-open-list` | normal, select | can be followed by another action | Show the running terminals |
| `terminal-rename` | normal, select | last unless an argument supplies the prompt input | Name the active terminal session |
| `terminal-leave` | normal, select | can be followed by another action | Show this pane's buffer again |
| `terminal-copy-output` | normal, select | can be followed by another action | Open this terminal's output as a buffer |
| `terminal-send` | normal, select | can be followed by another action | Send the selection to a terminal |

#### Help actions

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `show-help` | normal, select | can be followed by another action | Open general or contextual Runyte help |

#### Colon commands

These accept the same arguments as the command palette, supplied separately
as `{ action: name, arg: text }`. They are available in Normal and
Select modes. Prompt and background effects must end a sequence; supplying prompt input removes only that prompt. Required arguments must be supplied.

| Command | Usage | Description |
| --- | --- | --- |
| `describe-key` | `describe-key` | Inspect a key without executing it |
| `browse-actions` | `browse-actions` | Browse action descriptors and default keys |
| `copy-key-binding` | `copy-key-binding` | Copy the last inspected binding as YAML |
| `cd` | `cd [path]` | Change directory, moving to its workspace when it belongs to another |
| `init` | `init [directory]` | Make a directory a workspace and move to it |
| `session-attach` | `session-attach <workspace>` | Attach to another workspace's persistent session |
| `session-list` | `session-list` | Session manager and native controls |
| `session-stop` | `session-stop [workspace]` | Stop a clean persistent session |
| `session-rename` | `session-rename <workspace> <name>` | Rename a persistent session |
| `session-clean` | `session-clean` | Clean verified stopped session history |
| `diff-disk` | `diff-disk` | Compare the active file buffer with a fresh disk observation |
| `diff-remote` | `diff-remote` | Compare the active provider document with a fresh read-only remote snapshot |
| `diff-this` | `diff-this` | Compare this buffer with the next one marked |
| `diff-off` | `diff-off` | Close the comparison this buffer is part of |
| `format` | `format` | Format the active buffer with its language server |
| `grammar` | `grammar [runyte]` | Report the active Runyte editing grammar |
| `pipe` | `pipe <shell-command>` | Pipe each selection through a shell command |
| `pipe-cancel` | `pipe-cancel` | Cancel the workspace shell pipe |
| `plugins` | `plugins` | Inspect configured plugins and their state |
| `plugin-stop` | `plugin-stop <configured-id>` | Stop a plugin or cancel its pending restart |
| `plugin-restart` | `plugin-restart <configured-id>` | Restart a configured plugin after cleanup |
| `config-reload` | `config-reload` | Re-read the loaded configuration file without restarting |
| `write-config` | `write-config [path]` | Write the effective configuration to a new file |
| `write-config!` | `write-config! [path]` | Write the effective configuration, replacing any existing file |
| `service-health` | `service-health` | Inspect optional editor service health |
| `log-open` | `log-open` | Open this process's diagnostic log |
| `notifications` | `notifications` | Open the retained notification history |
| `path` | `path` | Show the active buffer's absolute path |
| `close` | `close` | Close the active buffer without changing the pane layout |
| `close!` | `close!` | Close the active buffer and discard its unsaved text |
| `git-compare` | `git-compare` | Compare with this branch or worktree |
| `git-branches` | `git-branches` | Open the local and remote branch list |
| `git-network` | `git-network` | Open or refresh the cached commit network |
| `git-network-head` | `git-network-head` | Restrict the commit network to current HEAD |
| `git-network-all` | `git-network-all` | Show all cached refs and HEAD in the commit network |
| `git-network-choose-ref` | `git-network-choose-ref` | Choose a captured full ref for the commit network |
| `git-network-ref` | `git-network-ref <full-ref>` | Restrict the commit network to an exact full ref |
| `git-log` | `git-log` | Open the Git log, or refresh it from its first page |
| `git-search-commits` | `git-search-commits` | Fuzzy-search commits reachable from HEAD by message, object ID, author, or date |
| `git-blame` | `git-blame` | Show attribution for the primary line using live buffer text |
| `git-blame-file` | `git-blame-file` | Open full-file attribution using live buffer text |
| `git-worktrees` | `git-worktrees` | Open or refresh the repository worktree list |
| `git-commit` | `git-commit` | Write a message and commit what is staged |
| `git-diff` | `git-diff` | Show the active file's unstaged diff |
| `git-diff-side-by-side` | `git-diff-side-by-side` | Compare the two complete versions of the active file |
| `git-discard` | `git-discard` | Throw away a file's uncommitted changes, after a confirmation |
| `git-index` | `git-index` | Review everything staged for the next commit |
| `git-cancel` | `git-cancel` | Stop the active Git operation and reconcile uncertain mutations |
| `git-status` | `git-status` | Open the changed-file list |
| `git-stashes` | `git-stashes` | Open or refresh the bounded stash list |
| `git-stash-tracked` | `git-stash-tracked <name>` | Confirm a tracked-worktree snapshot while keeping the index applied |
| `git-stash-all` | `git-stash-all <name>` | Confirm a named stash of tracked worktree and index changes |
| `git-stash-untracked` | `git-stash-untracked <name>` | Confirm a named stash including untracked files |
| `git-stash-apply` | `git-stash-apply` | Confirm applying the selected stash without dropping it |
| `git-stash-drop` | `git-stash-drop` | Confirm dropping the selected stash |
| `git-stage-hunk` | `git-stage-hunk` | Stage the exact hunk under the cursor |
| `git-unstage-hunk` | `git-unstage-hunk` | Unstage the exact hunk under the cursor |
| `git-stage-lines` | `git-stage-lines` | Stage the supported saved source-line selection |
| `git-stage` | `git-stage` | Stage the active file |
| `git-unstage` | `git-unstage` | Unstage the active file |
| `git-refresh` | `git-refresh` | Refresh Git state or retry failed repository discovery |
| `lsp-trust` | `lsp-trust` | Choose LSP permission for this workspace |
| `agent-channel` | `agent-channel` | Show this workspace's agent channel |
| `mcp` | `mcp [identity]` | Review MCP permissions and connected agents |
| `lsp-restart` | `lsp-restart [language]` | Restart stopped language servers |
| `lsp-status` | `lsp-status` | Report language server state |
| `open` | `open <path>` | Open a file or directory in the active pane |
| `detach` | `detach` | Detach the TUI from its persistent session |
| `quit` | `quit` | Close the active pane and its unique buffer, or quit from the last pane |
| `quit!` | `quit!` | Discard a unique buffer and close its pane, or force quit from the last pane |
| `quit-all` | `quit-all` | Quit if all buffers are saved and no terminal is running |
| `quit-all!` | `quit-all!` | Quit and discard unsaved buffers, but never terminate terminals |
| `quit-all!!` | `quit-all!!` | Discard unsaved buffers, terminate this workspace's terminals and quit |
| `quit-here` | `quit-here` | Quit and hand the active directory to the shell wrapper |
| `quit-here!` | `quit-here!` | Discard changes, quit, and hand the active directory to the shell wrapper |
| `reload` | `reload` | Reload the active view |
| `resize` | `resize` | Preview pane resizing and extension; Enter applies, Escape cancels |
| `write-quit` | `write-quit` | Write the active buffer, then close its pane or quit if it is the last pane |
| `write-buffer-close` | `write-buffer-close` | Write and close the active buffer without changing the pane layout |

#### Scoped actions

Use an action name or null in `keys.bind.<mode>`. Named modes also support
the applicable shared actions above. These additional actions belong to the
indicated buffer owner; raw UI input remains under `keys.ui`.

##### global

- `list-accept`: list
- `list-backspace`: list
- `list-clear-filter`: list
- `list-close`: list
- `list-close-target`: list
- `list-first`: list
- `list-last`: list
- `list-next`: list
- `list-open-actions`: list
- `list-page-down`: list
- `list-page-up`: list
- `list-previous`: list
- `list-toggle-preview`: list

##### media

- `media-actual-size`: normal, select
- `media-back`: normal, select
- `media-copy-selection`: normal, select
- `media-fit`: normal, select
- `media-move-down`: normal, select
- `media-move-up`: normal, select
- `media-pan-left`: normal, select
- `media-pan-right`: normal, select
- `media-select-all`: normal, select
- `media-zoom-in`: normal, select
- `media-zoom-out`: normal, select

##### pdf-pages

- `media-back`: normal, select
- `media-show-page`: normal, select

##### directory

- `directory-choose-order`: normal, select
- `directory-open-entry`: normal, select
- `directory-open-finder`: normal, select
- `directory-open-parent`: normal, select
- `directory-open-system`: normal, select
- `directory-open-workspace`: normal, select
- `directory-paste`: normal, select
- `directory-toggle-details`: normal, select
- `directory-toggle-hidden-files`: normal, select
- `session-open-explorer`: normal, select
- `terminal-open-directory-root`: normal, select

##### directory-tree

- `directory-toggle-hidden-files`: normal, select
- `directory-tree-close`: normal, select
- `directory-tree-collapse`: normal, select
- `directory-tree-delete`: normal, select
- `directory-tree-down`: normal, select
- `directory-tree-expand`: normal, select
- `directory-tree-first`: normal, select
- `directory-tree-last`: normal, select
- `directory-tree-legend`: normal, select
- `directory-tree-move`: normal, select
- `directory-tree-new`: normal, select
- `directory-tree-open`: normal, select
- `directory-tree-open-horizontal`: normal, select
- `directory-tree-open-vertical`: normal, select
- `directory-tree-page-down`: normal, select
- `directory-tree-page-up`: normal, select
- `directory-tree-pane-1`: normal, select
- `directory-tree-pane-2`: normal, select
- `directory-tree-pane-3`: normal, select
- `directory-tree-pane-4`: normal, select
- `directory-tree-pane-5`: normal, select
- `directory-tree-pane-6`: normal, select
- `directory-tree-pane-7`: normal, select
- `directory-tree-pane-8`: normal, select
- `directory-tree-pane-9`: normal, select
- `directory-tree-refresh`: normal, select
- `directory-tree-rename`: normal, select
- `directory-tree-search`: normal, select
- `directory-tree-search-next`: normal, select
- `directory-tree-search-previous`: normal, select
- `directory-tree-select-first-child`: normal, select
- `directory-tree-select-parent`: normal, select
- `directory-tree-up`: normal, select

##### settings

- `activate-setting`: normal, select
- `override-setting-language`: normal, select
- `override-setting-pattern`: normal, select
- `remove-setting-override`: normal, select

##### git-status

- `git-merge-abort`: normal, select
- `git-merge-continue`: normal, select
- `git-open-changed-file`: normal, select
- `git-pull-branch`: normal, select
- `git-push-branch`: normal, select
- `git-stage-all-changed-files`: normal, select

##### git-branches

- `git-checkout-branch`: normal, select
- `git-create-branch`: normal, select
- `git-create-worktree`: normal, select
- `git-delete-branch`: normal, select
- `git-fetch-branch`: normal, select
- `git-merge-branch`: normal, select
- `git-pull-branch`: normal, select
- `git-push-branch`: normal, select

##### git-conflicts

- `git-conflict-file-current`: normal, select
- `git-conflict-file-other`: normal, select
- `git-conflict-resolve`: normal, select
- `git-conflict-resolve-literal-markers`: normal, select
- `git-conflict-sides`: normal, select
- `git-merge-abort`: normal, select
- `git-merge-continue`: normal, select
- `git-open-conflict`: normal, select

##### git-conflict-file

- `git-conflict-file-current`: normal, select
- `git-conflict-file-other`: normal, select
- `git-conflict-keep-current`: normal, select
- `git-conflict-list`: normal, select
- `git-conflict-next`: normal, select
- `git-conflict-previous`: normal, select
- `git-conflict-resolve`: normal, select
- `git-conflict-resolve-literal-markers`: normal, select
- `git-conflict-sides`: normal, select
- `git-conflict-take-other`: normal, select

##### git-conflict-markdown

- `git-conflict-file-current`: normal, select
- `git-conflict-file-other`: normal, select
- `git-conflict-keep-current`: normal, select
- `git-conflict-list`: normal, select
- `git-conflict-next`: normal, select
- `git-conflict-previous`: normal, select
- `git-conflict-resolve`: normal, select
- `git-conflict-resolve-literal-markers`: normal, select
- `git-conflict-sides`: normal, select
- `git-conflict-take-other`: normal, select
- `toggle-markdown-render`: normal, select

##### git-merge-review

- `git-review-approve`: normal
- `git-review-cancel`: normal
- `git-review-enter`: normal
- `git-review-left`: normal
- `git-review-next`: normal
- `git-review-previous`: normal
- `git-review-right`: normal

##### git-merge-detail

- `git-review-back`: normal
- `git-review-cancel`: normal
- `git-review-enter`: normal
- `git-review-left`: normal
- `git-review-next`: normal
- `git-review-previous`: normal
- `git-review-right`: normal

##### git-merge-input

- `git-review-cancel`: normal
- `git-review-enter`: normal
- `git-review-left`: normal
- `git-review-next`: normal
- `git-review-previous`: normal
- `git-review-right`: normal

##### terminal

- `split-horizontal`: insert
- `split-vertical`: insert
- `terminal-clipboard-paste-text`: normal, select
- `terminal-enter-input`: normal, select
- `terminal-paste`: normal, select
- `terminal-search`: normal, select
- `terminal-search-next`: normal, select
- `terminal-search-previous`: normal, select
- `terminal-search-regex`: normal, select
- `terminal-undo-paste`: normal, select
- `toggle-fullscreen`: insert
- `toggle-zen`: insert

##### git-worktrees

- `git-create-new-worktree`: normal, select
- `git-open-worktree`: normal, select
- `git-remove-worktree`: normal, select

##### git-log

- `git-log-next-page`: normal, select
- `git-log-previous-page`: normal, select
- `git-open-commit`: normal, select

##### git-network

- `git-network-next-page`: normal, select
- `git-network-previous-page`: normal, select
- `git-open-commit`: normal, select

##### git-blame

- `git-open-commit`: normal, select

##### workspace-search

- `search-open-workspace-result`: normal, select

##### markdown

- `toggle-markdown-render`: normal, select

##### session-manager

- `list-accept`: list
- `list-backspace`: list
- `list-clear-filter`: list
- `list-close`: list
- `list-close-target`: list
- `list-first`: list
- `list-last`: list
- `list-next`: list
- `list-open-actions`: list
- `list-page-down`: list
- `list-page-up`: list
- `list-previous`: list
- `list-toggle-preview`: list
- `session-open-destinations`: list
- `session-open-directory`: list


#### Action contracts

Generated from built-in descriptors. Effects include owned continuations and specialized views; remaining prompt and background effects require the last step. Key-program arguments replace the listed operands or prompts; palette spellings remain separate.

| Action | Effects | Count | Uses register | Supports extend | Character operands | Palette argument | Key-program argument | View differences | Classification |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `fit-selection-for-normal-mode` | selection, view | not accepted | false | false | none | none | palette argument, if any | terminal: Fit the live or captured review selection for Normal mode without entering it | atomic operation |
| `mode-enter` | mode | not accepted | false | false | none | none | palette argument, if any |  | mode action |
| `enter-normal-mode` | mode, selection | not accepted | false | false | none | none | palette argument, if any | terminal: Leave child input for live Normal mode; otherwise capture review and collapse its selection | pending stage 5 split; then mode action |
| `open-command-palette` | prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `move-left` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the cursor in captured output; media: Pan horizontally, or extend the media selection in Select mode | atomic operation |
| `move-right` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the cursor in captured output; media: Pan horizontally, or extend the media selection in Select mode | atomic operation |
| `move-up` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the cursor in captured output; media: Pan vertically, change PDF page at fit, or extend the media selection | atomic operation |
| `move-down` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the cursor in captured output; media: Pan vertically, change PDF page at fit, or extend the media selection | atomic operation |
| `move-line-start` | selection | not accepted | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-line-end` | selection | not accepted | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-first-non-whitespace` | selection | not accepted | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-file-start` | selection, view | interpreted as an address or extent | false | true | none | none | palette argument, if any | terminal: Move to the captured output boundary; media: Select a PDF page by address or move to its first or last page | atomic operation |
| `move-file-end` | selection, view | interpreted as an address or extent | false | true | none | none | palette argument, if any | terminal: Move to the captured output boundary; media: Select a PDF page by address or move to its first or last page | atomic operation |
| `move-word-forward` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-word-backward` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-word-end` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-long-word-forward` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-long-word-backward` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `move-long-word-end` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `goto-next-paragraph` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `goto-previous-paragraph` | selection | repeats | false | true | none | none | palette argument, if any | terminal: Move or extend the review selection within captured output | atomic operation |
| `find-next-char` | character input, selection | not accepted | false | true | one character | none | one Unicode scalar; replaces the character operand | terminal: Find the supplied character within captured output | atomic operation |
| `find-previous-char` | character input, selection | not accepted | false | true | one character | none | one Unicode scalar; replaces the character operand | terminal: Find the supplied character within captured output | atomic operation |
| `find-till-next-char` | character input, selection | not accepted | false | true | one character | none | one Unicode scalar; replaces the character operand | terminal: Find the supplied character within captured output | atomic operation |
| `find-till-previous-char` | character input, selection | not accepted | false | true | one character | none | one Unicode scalar; replaces the character operand | terminal: Find the supplied character within captured output | atomic operation |
| `page-up` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move the review cursor by a page of captured output; media: Move to the previous or next PDF page | atomic operation |
| `page-down` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move the review cursor by a page of captured output; media: Move to the previous or next PDF page | atomic operation |
| `half-page-up` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move the review cursor by half a page of captured output; media: Move to the previous or next PDF page; directory-tree: Move the selected tree row by half a page | atomic operation |
| `half-page-down` | selection, view | repeats | false | true | none | none | palette argument, if any | terminal: Move the review cursor by half a page of captured output; media: Move to the previous or next PDF page; directory-tree: Move the selected tree row by half a page | atomic operation |
| `enter-insert-mode` | mode, selection | not accepted | false | false | none | none | palette argument, if any |  | pending stage 5 split; then mode action |
| `enter-replace-mode` | mode, selection | not accepted | false | false | none | none | palette argument, if any |  | pending stage 5 split; then mode action |
| `append-after` | mode, selection | not accepted | false | false | none | none | palette argument, if any |  | mode action |
| `insert-line-start` | mode, selection | not accepted | false | false | none | none | palette argument, if any |  | mode action |
| `insert-line-end` | mode, selection | not accepted | false | false | none | none | palette argument, if any |  | mode action |
| `open-line-below` | text, mode, selection | not accepted | false | false | none | none | palette argument, if any |  | mode action |
| `open-line-above` | text, mode, selection | not accepted | false | false | none | none | palette argument, if any |  | mode action |
| `replace-char` | character input, text | not accepted | false | false | one character | none | one Unicode scalar; replaces the character operand |  | atomic operation |
| `surround-add` | character input, text | not accepted | false | false | one character | none | one Unicode scalar; replaces the character operand |  | atomic operation |
| `surround-replace` | character input, text | not accepted | false | false | two characters | none | two Unicode scalars (old, new); replaces both operands |  | atomic operation |
| `surround-delete` | character input, text | not accepted | false | false | one character | none | one Unicode scalar; replaces the character operand |  | atomic operation |
| `toggle-case` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `selection-undo` | selection, mode | repeats | false | false | none | none | palette argument, if any | terminal: Restore the previous or next terminal review selection | decision: Restores the selection and mode together from the chosen history entry. |
| `selection-redo` | selection, mode | repeats | false | false | none | none | palette argument, if any | terminal: Restore the previous or next terminal review selection | decision: Restores the selection and mode together from the chosen history entry. |
| `undo` | text | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `redo` | text | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `yank` | register | not accepted | true | false | none | none | palette argument, if any | terminal: Copy the review selection to the selected register and retain its selection and mode; directory: Copy selected entries and record a directory copy payload | atomic operation |
| `yank-line` | register | not accepted | true | false | none | none | palette argument, if any | terminal: Copy whole review rows to the selected register, preserving selection and mode; directory: Copy selected entry lines and record a directory copy payload | atomic operation |
| `paste-after-or-replace` | text, selection | interpreted as an address or extent | true | false | none | none | palette argument, if any |  | decision: Chooses insertion or replacement independently for each selection range. |
| `paste-after` | text, selection | interpreted as an address or extent | true | false | none | none | palette argument, if any | directory: Stage copies or moves from a directory register into the listing | atomic operation |
| `replace-with-register` | text | not accepted | true | false | none | none | palette argument, if any | directory: Replace listing text from a text register; directory move/copy payloads are refused because entries cannot replace a text selection | atomic operation |
| `paste-before` | text, selection | interpreted as an address or extent | true | false | none | none | palette argument, if any | directory: Stage copies or moves from a directory register into the listing | atomic operation |
| `indent` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `unindent` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `toggle-comments` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `cut-selection` | register, text, selection | not accepted | true | false | none | none | palette argument, if any | directory: Record selected entries as a move payload without removing files; delete or change the listing selection | decision: Chooses linewise register content or directory move payload and coordinates deletion; independent yank/delete steps cannot retain that ownership. |
| `cut-selection-for-change` | register, text, selection | not accepted | true | false | none | none | palette argument, if any | directory: Record selected entries as a move payload without removing files; delete or change the listing selection | decision: Chooses operative-span register content or directory move payload and coordinates deletion; independent yank/delete steps cannot retain that ownership. |
| `delete-selection` | text | not accepted | false | false | none | none | palette argument, if any |  | decision: Delete and change place the caret and treat whole rows differently. |
| `delete-selection-for-change` | text | not accepted | false | false | none | none | palette argument, if any |  | decision: Delete and change place the caret and treat whole rows differently. |
| `enter-select-mode` | mode, selection | not accepted | false | false | none | none | palette argument, if any | terminal: Enter Select mode over captured terminal output; media: Begin selection of PDF text or an image region | pending stage 5 split; then mode action |
| `select-line` | selection | repeats | false | false | none | none | palette argument, if any | terminal: Select or extend captured output with whole-line extent, preserving mode | atomic operation |
| `extend-line-above` | selection | repeats | false | false | none | none | palette argument, if any | terminal: Select or extend captured output with whole-line extent, preserving mode | atomic operation |
| `extend-line-below` | selection | repeats | false | false | none | none | palette argument, if any | terminal: Select or extend captured output with whole-line extent, preserving mode | atomic operation |
| `select-line-up` | selection | repeats | false | false | none | none | palette argument, if any | terminal: Select or extend captured output with whole-line extent, preserving mode | atomic operation |
| `select-all` | selection | not accepted | false | false | none | none | palette argument, if any | terminal: Select all captured terminal output | atomic operation |
| `collapse-selection` | selection | not accepted | false | false | none | none | palette argument, if any | terminal: Collapse the review selection; media: Retain the native media selection; this surface has no document caret to collapse | atomic operation |
| `flip-selection` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `syntax-expand-selection` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `syntax-shrink-selection` | selection, mode | repeats | false | false | none | none | palette argument, if any |  | decision: Restores selection and mode from expansion history that is not visible between program steps. |
| `select-syntax-parent` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-syntax-child` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-previous-syntax-sibling` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-next-syntax-sibling` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-syntax-function` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-syntax-function` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-syntax-class` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-syntax-class` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-syntax-parameter` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-syntax-parameter` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-parentheses` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-parentheses` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-square-brackets` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-square-brackets` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-braces` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-braces` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-angle-brackets` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-angle-brackets` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-double-quotes` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-double-quotes` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-single-quotes` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-single-quotes` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-backticks` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-backticks` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-word` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-word` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-long-word` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-long-word` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-paragraph` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-paragraph` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-around-closest-delimiter` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `select-inside-closest-delimiter` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-previous-syntax-function` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-next-syntax-function` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-previous-syntax-class` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-next-syntax-class` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-previous-syntax-parameter` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-next-syntax-parameter` | selection | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `syntax-document-outline` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `syntax-toggle-fold` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `syntax-fold-all` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `syntax-unfold-all` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `split-selection-at-line-ends` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `split-selection-at-line-starts` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `keep-primary-selection` | selection | not accepted | false | false | none | none | palette argument, if any | terminal: Keep only the primary review selection | atomic operation |
| `remove-primary-selection` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `copy-selection-down` | selection | not accepted | false | false | none | none | palette argument, if any | terminal: Copy the review cursor to the adjacent captured output line | atomic operation |
| `copy-selection-up` | selection | not accepted | false | false | none | none | palette argument, if any | terminal: Copy the review cursor to the adjacent captured output line | atomic operation |
| `copy-selection-down-padded` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `copy-selection-up-padded` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `rotate-selection-forward` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `rotate-selection-backward` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `rotate-selection-contents-forward` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `rotate-selection-contents-backward` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `keep-matching-selections` | prompt, selection | not accepted | false | false | none | none | nonempty pattern; replaces the prompt |  | atomic operation |
| `remove-matching-selections` | prompt, selection | not accepted | false | false | none | none | nonempty pattern; replaces the prompt |  | atomic operation |
| `align-selections` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `trim-selections` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `trim-trailing-whitespace` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `hard-wrap` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `reflow` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `insert-text` | text, selection | not accepted | false | false | none | none | nonempty literal text (up to 4096 bytes); inserts at each caret |  | atomic operation |
| `join-selections` | prompt, text | not accepted | false | false | none | none | literal delimiter (may be empty); replaces the prompt |  | atomic operation |
| `join-lines` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `format-table` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search` | prompt, search state, selection, mode | not accepted | false | false | none | none | nonempty pattern; replaces the prompt |  | decision: Owns the search prompt and enters Select only when its accepted search succeeds (decision 30). |
| `search-regex` | prompt, search state, selection, mode | not accepted | false | false | none | none | nonempty pattern; replaces the prompt |  | decision: Owns the regular-expression search prompt and its selection-mode completion (decision 30). |
| `search-next` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-previous` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-extend-next` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-extend-previous` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-select-all-matches` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-set-from-selection` | search state | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-show-page` | view, mode, selection | not accepted | false | false | none | none | palette argument, if any | pdf-pages: Display the selected PDF page and enter Normal mode | decision: Chooses and presents a native media page with its corresponding selection and mode. |
| `media-zoom-in` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-zoom-out` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-fit` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-actual-size` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-pan-left` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-pan-right` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-move-up` | view, selection | repeats | false | true | none | none | palette argument, if any |  | decision: Pans zoomed or non-PDF media, but changes PDF pages at fit zoom; the native viewport owns that decision (decision 31). |
| `media-move-down` | view, selection | repeats | false | true | none | none | palette argument, if any |  | decision: Pans zoomed or non-PDF media, but changes PDF pages at fit zoom; the native viewport owns that decision (decision 31). |
| `media-copy-selection` | register | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `media-back` | view, mode, selection | not accepted | false | false | none | none | palette argument, if any | media: Enter Normal mode, then fit media, clear selection or leave preview; pdf-pages: Collapse a Select-mode page selection, otherwise open the parent explorer | decision: Fits zoomed media, otherwise clears native selection, otherwise leaves preview; authoritative native state chooses the branch (decision 31). |
| `media-select-all` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `align-view-center` | view | not accepted | false | false | none | none | palette argument, if any | media: Center the image or PDF in its pane | atomic operation |
| `align-view-top` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `align-view-bottom` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `align-view-middle` | view | not accepted | false | false | none | none | palette argument, if any | media: Center the image or PDF in its pane | atomic operation |
| `scroll-view-down` | view | repeats | false | false | none | none | palette argument, if any | terminal: Capture review and scroll its output; media: Pan the image or PDF vertically | atomic operation |
| `scroll-view-up` | view | repeats | false | false | none | none | palette argument, if any | terminal: Capture review and scroll its output; media: Pan the image or PDF vertically | atomic operation |
| `goto-window-top` | selection | not accepted | false | true | none | none | palette argument, if any | terminal: Move the review cursor within the visible captured output; directory-tree: Select the tree row at the top, middle or bottom of the view | atomic operation |
| `goto-window-center` | selection | not accepted | false | true | none | none | palette argument, if any | terminal: Move the review cursor within the visible captured output; directory-tree: Select the tree row at the top, middle or bottom of the view | atomic operation |
| `goto-window-bottom` | selection | not accepted | false | true | none | none | palette argument, if any | terminal: Move the review cursor within the visible captured output; directory-tree: Select the tree row at the top, middle or bottom of the view | atomic operation |
| `goto-file` | file, view, prompt, background work | not accepted | false | false | none | none | palette argument, if any | terminal: Open a path or URL inferred from the review selection and terminal directory | atomic operation |
| `goto-word` | prompt, selection | not accepted | false | false | none | none | palette argument, if any | terminal: Label visible captured output words and jump to one; directory-tree: Label visible tree entries and select one | atomic operation |
| `toggle-soft-wrap` | view, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `toggle-whitespace` | view, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `preview-document` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `toggle-markdown-render` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `toggle-focus` | view, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `toggle-zen` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Restores the previous editor mode when leaving directory-tree focus before changing the window presentation. |
| `toggle-fullscreen` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Restores the previous editor mode when leaving directory-tree focus before changing the window presentation. |
| `directory-open` | file, view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-toggle` | pane, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-focus` | pane, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-up` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-down` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-collapse` | view, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-select-parent` | view, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-expand` | view, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-select-first-child` | view, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-paste` | text, selection | interpreted as an address or extent | true | false | none | none | palette argument, if any | directory: Stage copies or moves from a directory register into the listing | decision: Stages directory copy or move payloads beside entry rows, or pastes a text register using each range's insertion/replacement decision. |
| `directory-tree-search` | prompt, selection, view, search state | not accepted | false | false | none | none | nonempty pattern; replaces the prompt | directory-tree: Prompt to search visible tree entries | atomic operation |
| `directory-tree-search-next` | selection, view | not accepted | false | false | none | none | palette argument, if any | directory-tree: Select the next or previous matching tree entry | atomic operation |
| `directory-tree-search-previous` | selection, view | not accepted | false | false | none | none | palette argument, if any | directory-tree: Select the next or previous matching tree entry | atomic operation |
| `directory-tree-first` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-last` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-page-up` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-page-down` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-open` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-close` | pane, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-refresh` | file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-legend` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-open-vertical` | file, view, mode, prompt, background work, pane | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-open-horizontal` | file, view, mode, prompt, background work, pane | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-1` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-2` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-3` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-4` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-5` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-6` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-7` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-8` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-pane-9` | file, view, mode, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-tree-new` | prompt, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-rename` | prompt, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-delete` | prompt, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-tree-move` | prompt, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-open-working-directory` | file, view, prompt | not accepted | false | false | none | explorer [path] | palette argument, if any |  | atomic operation |
| `open-file-picker` | view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `open-all-files-picker` | view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `open-path-file-picker` | prompt, view, background work | not accepted | false | false | none | file-picker-path [path] | palette argument, if any |  | atomic operation |
| `directory-open-file-picker` | view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-open-fuzzy-grep` | view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-open-fuzzy-grep` | view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-open-entry` | file, view, mode, selection, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-open-parent` | file, view, mode, selection, prompt | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `directory-refresh` | file, view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-toggle-hidden-files` | view, configuration | not accepted | false | false | none | none | palette argument, if any | directory-tree: Show or hide hidden tree entries | atomic operation |
| `directory-toggle-details` | view, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-choose-order` | prompt, view, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-open-system` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-open-finder` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-open-changed-file` | background work, file, view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-stage-all-changed-files` | background work, file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-checkout-branch` | background work, file, view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-create-branch` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-delete-branch` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-merge-branch` | background work, file, view, prompt, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-conflicts` | background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-open-conflict` | background work, file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-merge-continue` | background work, file, view, prompt, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-merge-abort` | prompt, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-conflict-next` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-conflict-previous` | selection, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-conflict-keep-current` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-conflict-take-other` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-conflict-sides` | background work, file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-conflict-resolve-literal-markers` | prompt, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-conflict-resolve` | prompt, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-conflict-list` | background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-conflict-file-current` | prompt, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-conflict-file-other` | prompt, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-review-approve` | view, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-review-cancel` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-review-back` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-review-next` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-review-previous` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-review-left` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-review-right` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-review-enter` | view, background work, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-fetch-branch` | background work, file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-pull-branch` | background work, file, view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-push-branch` | background work, file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-open-worktree` | background work, file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-create-worktree` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-create-new-worktree` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-remove-worktree` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-network-next-page` | background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-network-previous-page` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-log-next-page` | background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-log-previous-page` | background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `git-open-commit` | background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `search-open-workspace-result` | view, file, selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `override-setting-language` | prompt, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `override-setting-pattern` | prompt, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `remove-setting-override` | configuration, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `activate-setting` | prompt, configuration | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `open-settings` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `open-theme-settings` | configuration, view | not accepted | false | false | none | theme [name] | palette argument, if any |  | atomic operation |
| `split-vertical` | pane, mode, file, prompt, background work | not accepted | false | false | none | vsplit [path] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `split-horizontal` | pane, mode, file, prompt, background work | not accepted | false | false | none | hsplit [path] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `save` | file, text, prompt, background work | not accepted | false | false | none | write [path] | palette argument, if any | directory: Review and apply edits to the directory listing as filesystem operations; commit-message: Review staged changes and submit the commit message | atomic operation |
| `force-save` | file, text, prompt, background work | not accepted | false | false | none | write! [path] | palette argument, if any | directory: Review and apply edits to the directory listing as filesystem operations; commit-message: Review staged changes and submit the commit message | atomic operation |
| `show-help` | view, mode | not accepted | false | false | none | help [topic] | palette argument, if any | directory-tree: Open directory-tree help | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `show-about` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `show-tutorial` | view, pane, mode | not accepted | false | false | none | tutorial [reset&#124;sessions&#124;next] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `focus-window-left` | pane, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `focus-window-down` | pane, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `focus-window-up` | pane, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `focus-window-right` | pane, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `next-window` | pane, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `swap-window` | pane, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `move-window-left` | pane | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `move-window-down` | pane | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `move-window-up` | pane | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `move-window-right` | pane | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `close-window` | pane, mode | not accepted | false | false | none | none | palette argument, if any | directory-tree: Close the directory tree and restore pane focus | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `only-window` | pane | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `equalize-windows` | pane | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `terminal-undo-paste` | terminal input | repeats | false | false | none | none | palette argument, if any | terminal: Ask the child to erase the last text Runyte sent, if it has not been submitted | atomic operation |
| `terminal-enter-input` | mode, view | not accepted | false | false | none | none | palette argument, if any | terminal: Return to live output and send subsequent input to the child | mode action |
| `terminal-paste` | terminal input | interpreted as an address or extent | true | false | none | none | palette argument, if any | terminal: Send register text to the child without changing mode | atomic operation |
| `terminal-clipboard-paste-text` | terminal input | interpreted as an address or extent | false | false | none | none | palette argument, if any | terminal: Send system clipboard text to the child without changing mode | atomic operation |
| `terminal-search` | prompt, search state, selection, view | not accepted | false | false | none | none | nonempty pattern; replaces the prompt | terminal: Prompt for a case-insensitive search of captured output | atomic operation |
| `terminal-search-regex` | prompt, search state, selection, view | not accepted | false | false | none | none | nonempty pattern; replaces the prompt | terminal: Prompt for a regular-expression search of captured output | atomic operation |
| `terminal-search-next` | selection, view | not accepted | false | false | none | none | palette argument, if any | terminal: Select the next or previous terminal review search match | atomic operation |
| `terminal-search-previous` | selection, view | not accepted | false | false | none | none | palette argument, if any | terminal: Select the next or previous terminal review search match | atomic operation |
| `terminal-open` | background work, view, mode | not accepted | false | false | none | terminal [command] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-open-file-directory` | background work, view, mode | not accepted | false | false | none | terminal-file-directory [command] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-open-directory-root` | background work, view, mode | not accepted | false | false | none | terminal-directory-root [command] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-open-selected-directory` | background work, view, mode | not accepted | false | false | none | terminal-selected-directory [command] | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-open-session-directory` | background work, view, mode | not accepted | false | false | none | terminal-session-directory <number&#124;name> | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-open-list` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `terminal-show` | view, mode | not accepted | false | false | none | terminal-show <number&#124;name> | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-rename` | prompt, configuration | not accepted | false | false | none | terminal-rename <name> | palette argument, if any |  | atomic operation |
| `terminal-leave` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-copy-output` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `terminal-send` | terminal input | not accepted | false | false | none | terminal-send [number&#124;name] | palette argument, if any |  | atomic operation |
| `delete-word-backward` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `delete-word-forward` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `delete-to-line-start` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `delete-to-line-end` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `delete-char-backward` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `delete-char-forward` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `insert-newline` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `insert-indent` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `insert-other-indent` | text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `commit-undo-checkpoint` | history | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-definition` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-declaration` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-type-definition` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-references` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `goto-implementation` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `new-buffer` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `open-navigator` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `previous-destination` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `session-previous` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-1` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-2` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-3` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-4` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-5` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-6` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-7` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-8` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-9` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-next-running` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-previous-running` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-open-directory` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-open-explorer` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `directory-open-workspace` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `open-buffer-picker` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-accept` | view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-open-actions` | view, prompt | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-backspace` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-next` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-previous` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-page-down` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-page-up` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-first` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-last` | selection | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-toggle-preview` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-close-target` | view, prompt, background work, text, file, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Closes the captured target only after its guards or confirmation succeed and reconciles the surviving destination mode. |
| `list-clear-filter` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `list-close` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-open-destinations` | file, view, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `search-workspace` | prompt, background work, view, mode | not accepted | false | false | none | none | nonempty pattern; replaces the prompt |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `search-workspace-regex` | prompt, background work, view, mode | not accepted | false | false | none | none | nonempty pattern; replaces the prompt |  | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `show-documentation` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `document-symbols` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `workspace-symbols` | prompt, background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `diagnostics` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `trigger-completion` | background work, prompt, text, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Enters Insert only when a language server is available, then owns the completion request and later text acceptance. |
| `jump-backward` | view, selection, mode | not accepted | false | false | none | none | palette argument, if any | terminal: Navigate retained jump history through the ordinary pane workflow | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `jump-forward` | view, selection, mode | not accepted | false | false | none | none | palette argument, if any | terminal: Navigate retained jump history through the ordinary pane workflow | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `jump-backward-buffer` | view, selection, mode | not accepted | false | false | none | none | palette argument, if any | terminal: Navigate retained jump history through the ordinary pane workflow | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `jump-forward-buffer` | view, selection, mode | not accepted | false | false | none | none | palette argument, if any | terminal: Navigate retained jump history through the ordinary pane workflow | decision: Activates or restores a view and its destination mode as one guarded workflow. |
| `rename-symbol` | background work, prompt, text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `code-action` | background work, prompt, text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `clipboard-paste-after` | text, selection | interpreted as an address or extent | false | false | none | none | palette argument, if any |  | atomic operation |
| `clipboard-paste-before` | text, selection | interpreted as an address or extent | false | false | none | none | palette argument, if any |  | atomic operation |
| `clipboard-yank` | register | not accepted | false | false | none | none | palette argument, if any | terminal: Copy the captured review selection to the system clipboard; media: Copy selected PDF text or an image region | atomic operation |
| `clipboard-paste` | text, selection | not accepted | false | false | none | none | palette argument, if any |  | decision: Chooses image insertion or text paste from clipboard content. |
| `register-select` | character input, register | not accepted | false | false | one character | none | one Unicode scalar; replaces the character operand |  | atomic operation |
| `macro-record` | character input, macro | not accepted | false | false | one character | none | one Unicode scalar; replaces the character operand |  | atomic operation |
| `macro-record-default` | macro | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `macro-stop-recording` | macro | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `macro-replay` | character input, macro, text, background work | repeats | false | false | one character | none | one Unicode scalar; replaces the character operand |  | atomic operation |
| `macro-replay-default` | macro, text, background work | repeats | false | false | none | none | palette argument, if any |  | atomic operation |
| `macro-list` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `shell-pipe` | prompt, background work, text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `match-bracket` | selection | not accepted | false | true | none | none | palette argument, if any |  | atomic operation |
| `cd` | file, configuration, prompt, background work, view, application | not accepted | false | false | none | cd [path] | palette argument, if any |  | atomic operation |
| `init` | file, configuration, background work, view, application | not accepted | false | false | none | init [directory] | palette argument, if any |  | atomic operation |
| `close` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `diff-disk` | file, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `diff-remote` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `diff-off` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `diff-this` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `close!` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `format` | background work, text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-compare` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-branches` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-blame` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-blame-file` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-cancel` | background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-commit` | prompt, background work, file, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-diff` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-diff-side-by-side` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-discard` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-index` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-log` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-network` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-network-head` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-network-all` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-network-ref` | background work, view, mode | not accepted | false | false | none | git-network-ref <full-ref> | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-network-choose-ref` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-search-commits` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-refresh` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-stage` | background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-status` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-stashes` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `git-stash-tracked` | prompt, background work, file | not accepted | false | false | none | git-stash-tracked <name> | palette argument, if any |  | atomic operation |
| `git-stash-all` | prompt, background work, file | not accepted | false | false | none | git-stash-all <name> | palette argument, if any |  | atomic operation |
| `git-stash-untracked` | prompt, background work, file | not accepted | false | false | none | git-stash-untracked <name> | palette argument, if any |  | atomic operation |
| `git-stash-apply` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-stash-drop` | prompt, background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-stage-hunk` | background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-unstage-hunk` | background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-stage-lines` | background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-unstage` | background work, file | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `git-worktrees` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `config-reload` | configuration, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `write-config` | file | not accepted | false | false | none | write-config [path] | palette argument, if any |  | atomic operation |
| `write-config!` | file | not accepted | false | false | none | write-config! [path] | palette argument, if any |  | atomic operation |
| `grammar` | configuration, mode, selection | not accepted | false | false | none | grammar [runyte] | palette argument, if any |  | decision: Changes the editing grammar and reconciles its current mode and selection. |
| `lsp-trust` | prompt, configuration, background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `mcp` | prompt, configuration, background work | not accepted | false | false | none | mcp [identity] | palette argument, if any |  | atomic operation |
| `agent-channel` | background work, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `lsp-restart` | background work | not accepted | false | false | none | lsp-restart [language] | palette argument, if any |  | atomic operation |
| `log-open` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `lsp-status` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `notifications` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `open` | file, view, prompt, background work | not accepted | false | false | none | open <path> | palette argument, if any |  | atomic operation |
| `path` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `describe-key` | prompt, view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `browse-actions` | view, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Opens or updates a workflow view with the mode belonging to that view. |
| `copy-key-binding` | register | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `detach` | application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit` | pane, view, application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit!` | pane, view, application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit-all` | application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit-all!` | application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit-all!!` | application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit-here` | application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `quit-here!` | application | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `reload` | prompt, background work, file, text | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `resize` | pane, prompt, mode | not accepted | false | false | none | none | palette argument, if any |  | decision: Owns the resize prompt and its mode transition. |
| `service-health` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `pipe` | background work, text | not accepted | false | false | none | pipe <shell-command> | palette argument, if any |  | atomic operation |
| `pipe-cancel` | background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `plugins` | view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `plugin-stop` | background work | not accepted | false | false | none | plugin-stop <configured-id> | palette argument, if any |  | atomic operation |
| `plugin-restart` | background work | not accepted | false | false | none | plugin-restart <configured-id> | palette argument, if any |  | atomic operation |
| `write-quit` | file, text, view, application, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Owns the save continuation; closes only after successful save and preserves commit-message or changed-buffer guards. |
| `write-buffer-close` | file, text, view, prompt, background work | not accepted | false | false | none | none | palette argument, if any |  | decision: Owns the save continuation; closes only after successful save and preserves commit-message or changed-buffer guards. |
| `session-attach` | background work, view | not accepted | false | false | none | session-attach <workspace> | palette argument, if any |  | atomic operation |
| `session-list` | background work, view | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
| `session-stop` | background work | not accepted | false | false | none | session-stop [workspace] | palette argument, if any |  | atomic operation |
| `session-rename` | background work, configuration | not accepted | false | false | none | session-rename <workspace> <name> | palette argument, if any |  | atomic operation |
| `session-clean` | background work | not accepted | false | false | none | none | palette argument, if any |  | atomic operation |
<!-- action-reference:end -->

<!-- navigation -->
[Contents](README.md) · [Previous: Configuration](configuration.md) · [Next: Themes](themes.md)

`ui.macro-replay.cancel` stops cooperative macro replay. It defaults to Escape
with any modifiers and exact Ctrl-c. While replay is running, other physical
input remains ignored. Replacing this action removes both default aliases;
`[]` disables keyboard cancellation. The leader does not add a cancellation
alias. The progress message shows the effective keys.

`ui.hover.accept` opens the full documentation buffer from a long hover popup
(default: plain Enter). Short hovers and other keys keep dismissal followed
by normal input handling. Command prompts, completion and pending key sequences
keep precedence. Character operands and plain count digits remain structural.
Hover hints show only reachable aliases. The hover owner adds no leader alias. `[]` removes the expansion shortcut.

`ui.character-input.cancel` cancels a pending character operand, such as the
character after `f` or either operand of `m r`. It defaults to Escape with any
modifiers. Character keys, including modified characters, remain operands and
cannot be assigned to cancellation. Named keys and function keys are available.
Other non-character keys keep their existing “expected a character” result.
Removing cancellation does not make them valid operands. The owner adds no
leader alias.

`ui.grammar-prefix` supports `cancel` (Escape) and `back` (Backspace) while a
Normal/Select key sequence is pending. Cancellation clears its count. Backtracking
keeps it. Both default keys accept all modifiers. The same key can still start a
sequence when nothing is pending. Insert/Replace keep their existing prefix
fallback behavior. Character operands use their separate owner above.

`ui.key-hints` supports `next` and `previous`, defaulting to exact Ctrl-n/Ctrl-p
and plain Down/Up. Default arrows yield to registered continuations. Explicitly
configured arrows own pending hint input. Window-prefix hints in Insert/Replace
also support configured scrolling. Both frontends show effective scroll keys.

Prefix controls and hint scrolling are validated together against the final
binding map. Null removals can release conflicting bindings. Released control
keys can then appear inside configured sequences. If the final map still has a
collision, both of these UI owners return to defaults and binding edits are
compiled again against that policy. Other UI owners keep their configuration.
A key cannot belong to both prefix control and hint scrolling. Merge reviews
keep their separate scoped controls and do not pass through hint scrolling.

`ui.context-launcher.open-actions` replaces the Normal/Select action-menu shortcut
(default: plain Tab). Pending key sequences, character operands and the directory
tree keep precedence. Plain count digits remain structural. Replacing or
removing the shortcut releases Tab for `keys.bind` and scoped assignments.
Menu mnemonic letters keep their existing structural role.

When no native action is available, the launcher falls back. Without a ready
language server it reports that no actions are available. With one, it passes
the pressed key and count to the editor keymap. For a remapped launcher, assign
the desired editor fallback to that same key. It does not synthesize Tab. Help
prose and action tables show the effective key.

`ui.native-window` supports `font-increase`, `font-decrease`, and `paste-text`.
These actions run in the desktop window and use the same effective bindings as
its help. Defaults remain Ctrl-+/Ctrl-=, Ctrl--, and Ctrl-Shift-v (also Super-v
on macOS). For example:

```yaml
keys:
  ui:
    native-window:
      font-increase: [F8]
      font-decrease: [F7]
      paste-text: [Alt-v]
```

Plain text characters cannot be window shortcuts. Authored character chords yield
to structural operands and text owned by an active prompt or menu. Factory
shortcuts keep their historical precedence. Queued input keeps the bindings
from the frame where it was captured. Structural ownership is checked after
preceding editor input has been processed. Document previews still receive
navigation before window shortcuts, and pasting keeps its existing preview
dismissal behavior.

Native document previews use part of your `keys.bind.media` bindings:
movement and scrolling, pages and half-pages, document start/end, zoom, reset
zoom, copy, and return to source. Only a single implemented semantic action
executes locally. Action chains, plugin commands, and other commands continue
through the source buffer's actual mode and scope. A Media-only unsupported
command has no local preview meaning and is omitted from preview hints.

`ui.native-preview` exposes `back`, `copy`, `keep-open-and-continue`, and
`forward-pane`. Back defaults to Escape. Copy also accepts the existing
Ctrl/Super C variants. Keep-open keeps the existing Space, colon, and Ctrl-w
forwarding behavior. Forward-pane controls the compatibility Ctrl-arrow and
fast-pane-key bypasses. Effective source pane commands remain host-owned even
with `forward-pane: []`. Plain count digits remain structural.

Replacing Back or Copy removes its inherited Media aliases as well as its
native compatibility aliases. Explicit Media assignments for that action
remain. An explicit UI alias overrides local navigation at that key. A
host-owned pane or prefix family takes precedence, and help omits masked
aliases. If a source command shares a first key with preview navigation,
pressing that key returns to source, so the complete command reaches the
editor.

Queued preview input keeps the painted map and document identity. Returning to
source preserves subsequent queued typing in that same pane and source. Input
for an unrelated replacement preview is discarded. A configured keep-open action
that enters Insert/Replace or starts a character operand hands subsequent input
to that editor interaction until it finishes. The preview map is limited to 512
local/forwarding entries and 512 compatibility aliases. If it exceeds those
limits, the preview reports this and input returns to source. It never installs
a partial map.

`ui.pane-resize` owns keyboard layout previews. Its actions are `next` / `previous`
(Tab / Shift-Tab), `move-left/right/up/down` (h/l/k/j and arrows),
`extend-left/right/up/down` (H/L/K/J and Shift+arrows), `back` (u, undo),
`accept` (Enter), and `cancel` (Esc). The legend uses the effective bindings.
This owner inherits no leader alias. The `resize` command opens the preview;
previous directional resize command bindings must be updated to it.
