## Custom key bindings

The `keys` section configures keyboard actions and pane gestures:

| Key | What it does |
| --- | --- |
| `keys.leader` | Replaces `Space` as the leader prefix. |
| `keys.window` | Replaces `Ctrl-w` as the window prefix. |
| `keys.rebind` | Moves bindings Runyte already ships to other keys. |
| `keys.bind` | Assigns named actions to keys, per mode. Also removes bindings. |
| `keys.defaults` | `false` starts `bind` from an empty keyboard instead of Runyte's. See [Without the built-in keys](custom-key-bindings.md#without-the-built-in-keys). |
| `keys.pointer` | Replaces pane move/extend drag gestures. |
| `keys.ui` | Replaces completion, context-menu, and path-popup action bindings. |
| `keys.scopes` | Assigns or removes actions in individual editor views and result lists. |

- `keys` is read from the configuration file and from
  [`keybinds.d/`](configuration.md#themes-and-key-bindings-in-their-own-files) beside it; the
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

### View and list bindings

`keys.scopes.<scope>.<mode>` assigns one supported action name to a key, or
`null` to suppress that exact binding in that view. Other views and modes keep
their bindings. Suppression also hides inherited bindings; it does not reveal
the global action underneath, and does not remove longer sequences beginning
with the same key.

```yaml
keys:
  scopes:
    markdown:
      normal:
        j: move-up
        k: null
    global:
      list:
        F9: list-next
        Ctrl-n: null
    session-manager:
      list:
        F8: list-previous
```

Editor scope names are `markdown`, `directory`, `directory-tree`, `settings`,
`media`, `pdf-pages`, `help`, `commit-message`, `diff`, `workspace-search`,
`terminal`, and the Git scopes: `git-status`, `git-branches`, `git-conflicts`,
`git-conflict-file`, `git-conflict-markdown`, `git-merge-review`,
`git-merge-detail`, `git-merge-input`, `git-worktrees`, `git-comparison`,
`git-revision-diff`, `git-log`, `git-network`, `git-blame`, and `git-stash`.
Ordinary editor scopes support `normal`, `select`, `insert`, and `replace`.
The merge review scopes currently dispatch `normal` only. Terminal `replace`
is unavailable. Terminal `insert` admits terminal-safe mode, pane, clipboard,
and persistent-session actions. For example:

```yaml
keys:
  scopes:
    terminal:
      insert:
        F6: enter-normal-mode
        Ctrl-4: null
        'Ctrl-\': null
        'F7 h': focus-window-left
```

Ordinary keys still reach the child. An assigned action claims its first key
and any following sequence; an explicit persistent-session action also claims
its key outside a persistent session and reports when unavailable. A `null`
removes only the exact binding and its legacy capture policy. Longer surviving
bindings still claim their prefix. Removing `Ctrl-w` alone therefore does not
remove the window namespace. Unmodified Escape still reaches the child unless
explicitly assigned. Global custom text-editing bindings remain excluded from
Terminal Insert.

Use `global.list` for ordinary result-list bindings and `session-manager.list`
for overrides in the session manager. List bindings contain one key. List
navigation actions are `list-next`, `list-previous`, `list-page-down`,
`list-page-up`, `list-first`, `list-last`, `list-toggle-preview`,
`list-clear-filter`, and `list-close`. Use `list-accept` to activate a row,
`list-open-actions` to open its actions (or cycle tags / advance to the next row
when no action menu applies), and `list-backspace` to delete the last filter
character. These three actions do nothing in reports. Their inherited Enter,
Tab and Backspace bindings accept every modifier combination, preserving the
original behavior; scoped overrides replace or remove only the exact spelling.
For example, `Enter: null` leaves `Shift-Enter` available. Empty-filter leader
input follows the effective Escape action, including its removal. Nested action
menus own their input separately. Numbered session choices remain structural
input.

`:help key-actions` includes the scoped action names and their modes.
Actions must be supported by the selected view and mode. Scoped assignments
currently take one action name, without arguments or action lists. Editor
sequences may contain up to eight keys, including `Leader` and `Window`.
Context-action Tab, overlay leader dismissal,
and merge acknowledgment editing still belong to their existing input owners;
assignments or removals that those handlers would bypass are rejected.

Assignments apply simultaneously. Invalid scoped configuration rolls back the
**whole `scopes` section**, while `bind`, `ui`, and `pointer` remain independent.
A child override wins over inherited parents. If both Markdown and conflict-file
scopes override the same key, `git-conflict-markdown` must decide explicitly,
including by assigning `null`. Configuration fragments merge by scope, mode,
and key. Existing scoped defaults remain with `keys.defaults: false`; that
option still controls only global editor defaults.

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

Each action list replaces all its default bindings. `[]` removes them; omitted
actions retain defaults. UI actions are independent of `keys.defaults`.
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
Unchanged defaults retain their existing handling of additional modifiers.

The `context-actions`, `path-actions`, and `program-actions` owners support `next`, `previous`,
`accept`, and `cancel`. Context and path actions default to Down/j,
Up/k/Shift-Tab, Enter, and Tab/Esc/Ctrl-c respectively. Program actions use
Down/Ctrl-n and Up/Ctrl-p/Shift-Tab for navigation, Enter to accept, and
Tab/Esc/Ctrl-c to cancel. The `path` popup supports `open-actions` (Tab) and
`cancel` (Esc/Ctrl-c). These menu owners also inherit the effective leader as a
cancel key. Reassigning or removing `cancel` removes that leader binding too.

The nested `buffer-actions`, `terminal-actions`, and `session-actions` owners
support `next`, `previous`, `accept`, `back`, and `cancel`. Their navigation uses
Down/Ctrl-n and Up/Ctrl-p/Shift-Tab; Enter accepts, Tab goes back, and Esc/Ctrl-c
cancels. Both back and cancel close the menu. They are separate actions because
back, like acceptance, refuses a stale selected row; cancel can always dismiss
that stale menu. This distinction follows the actions when remapped. These
owners also inherit leader cancellation. Replacing cancel leaves back available;
replace or remove both actions to change every dismissal key.

For terminal and session actions requiring a second confirmation, the configured
accept key both arms and confirms. Navigation disarms the confirmation; an
unbound key does not. Acceptance still revalidates the captured target and
requires the current presented frame where that gate applies.

The `plugin-manager-actions` owner provides a transparent `cancel` action that
returns from the plugin action list to the manager, preserving its filter and
selection. Escape, Ctrl-c, and the empty-filter leader are its defaults. Other
input continues through `global.list`. This return action takes precedence over
scoped bindings in that list; hints and acceptance authority use the same order.
Replacing or removing return bindings exposes any underlying scoped action:
for example, removing this owner's cancel makes Escape perform `list-close`
unless you also change Escape in `global.list`. Removing its inherited leader
restores literal filter input without an implicit Escape translation.

The `confirmation` owner supports `accept` and `cancel` for file and directory
reloads, buffer and Git discard, and pull/rebase confirmation. Defaults are Enter
and Esc/Ctrl-c. `stash-confirmation` has the same actions but defaults to Enter
and Esc; Ctrl-c remains inert there. Both inherit leader cancellation only where
the existing overlay allows it (for example, an underlying nonempty filter
suppresses it). Replacing `cancel` also replaces inherited leader cancellation.
A remapped acceptance still revalidates the captured file, repository, or other
target, and requires the current presented frame wherever that check applies.

The `text-confirmation` owner handles branch switching/deletion and worktree
removal. It supports `accept`, `cancel`, `backspace`, `delete`, `move-left`,
`move-right`, `move-start`, and `move-end`. Editing defaults are Backspace,
Delete, Left, Right, Home, and End; prompt-style Ctrl-h/Ctrl-a aliases are not
inherited. Text and pasted names remain structural input. Remapping acceptance
does not bypass an exact-name requirement or change the authorization required
by the captured operation. Untyped branch/worktree decisions use the same owner,
but their editing actions remain inactive.

The `filesystem-confirmation` owner supports `accept`, `cancel`, `next`,
`previous`, `page-up`, `page-down`, `first`, and `last`. Enter applies the reviewed
plan with trash deletion; Esc/Ctrl-c cancel. Navigation defaults are Down/Ctrl-n,
Up/Ctrl-p, PageDown/Ctrl-d, PageUp/Ctrl-u, Home, and End. Plain `P` remains the
structural permanent-deletion choice; Shift-P does not confirm it. An inherited
leader takes precedence over that choice, and the hint reflects that precedence.
Plugin-owned plans retain their host execution and captured approval checks.

The `tree-destination` owner supports `accept` (Enter), `cancel` (Esc), and
`backspace`. Plain pane digits remain structural and still select immediately
when they identify one captured pane. `tree-delete` supports `cancel` (Enter or
Esc); y/Y and n/N remain structural yes/no choices. Default tree-delete keys
accept Shift/Meta/Hyper but ignore Control/Alt/Super; configured cancellation
matches its exact authored modifiers. Neither tree owner adds a Ctrl-c alias.
Both preserve conditional leader cancellation and captured targets.

Merge-review controls use `keys.scopes.git-merge-review`, `git-merge-detail`,
and `git-merge-input`. Acknowledgment editing uses `ui.merge-acknowledgment`
with `backspace`, `delete`, `move-left`, and `move-right`. Literal text remains
structural. Editing preserves the pending scoped key sequence and the existing
cursor behavior. Removing an editing alias permits that key to be assigned in
`git-merge-input`; configurations cannot advertise a scoped action hidden by
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
remain literal input. Choice fields use plain Left/Right and Space by default;
Space is the configurable `toggle-selection` action there. Text fields retain
Space as text. Picker Backspace removes one character; form editing retains
its grapheme-aware cursor and deletion behavior.

Path completion takes precedence only while candidates are available.
`accept` defaults to Tab; `accept-or-continue` defaults to Enter and completes
an unfinished path, otherwise allowing the form to handle Enter. Completion
Up/Down cycle candidates; modified navigation falls through to the form.
Repeating Tab may complete a path, while repeating conditional completion or
submission cannot submit a form. Remapped submission retains fresh-input,
presented-frame, field-validation, and captured-context checks. Completion
cannot lend submission authority to another input owner.

The `provider-reload` owner supports `next`, `previous`, `accept`, and `cancel`;
`provider-overwrite` supports `accept` and `cancel`. Both retain fresh physical
approval and captured document checks. Reload defaults to plain Down/Tab and
Up, exact Ctrl-n/Ctrl-p, and any-modifier BackTab; Shift-Tab remains inert.
Enter accepts, Esc or exact Ctrl-c cancels, and the leader also cancels.
When Enter is the leader, reload cancels while overwrite accepts, preserving
its original precedence. Repeated acceptance cannot approve either dialog.

The `context-review` owner controls permission-review navigation: `next`,
`previous`, `toggle-selection`, `accept`, `cancel`, `first`, `last`, `page-up`,
`page-down`, `half-page-up`, and `half-page-down`. Choice defaults are Down, Up,
Tab/Shift-Tab/BackTab, Enter, and Esc. Scrolling inherits the approved normal-mode
editor motions, with plain Home/End fallbacks. Configuring a scrolling action
replaces its inherited motions as well; an empty list disables them. Removed
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
selection preserved; directory cancellation restores the preceding editor mode.
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
choice keys other than j/k; path actions reserve r and s with any modifiers,
preserving their existing choice handling. An inherited leader keeps its old
cancellation precedence even if it matches a menu choice or navigation key.
Changing `cancel` restores the covered key's ordinary meaning. Hints show only
currently reachable shortcuts. Cancel in path actions returns to the path
popup; cancel there closes it.

Prompt owners are `command-prompt` (the command palette), `text-prompt`
(search, rename, settings and other text entry), `path-prompt` (Finder path
and directory-tree Move), and `program-prompt` (Open with). They do not inherit
leader cancellation: literal characters, including spaces, remain text.

All prompt owners support `accept`, `cancel`, `save`, `move-left`, `move-right`,
`move-start`, `move-end`, `word-left`, `word-right`, `delete-backward`,
`delete-forward`, `delete-word-backward`, `delete-to-start`, `delete-to-end`,
and `backspace`. The defaults retain Ctrl-c/s/b/f/a/e, Alt-b/f, Ctrl-h/d/w/u/k,
and Backspace respectively. `backspace` resets completion selection;
`delete-backward` (Ctrl-h) preserves it. Non-command prompts additionally
support `delete` (Delete), which resets completion selection;
`delete-forward` (Ctrl-d) preserves it. The command palette's physical Delete
key remains unbound by default.

Command, path and program prompts support `next` (Down) and `previous`
(Up/Shift-Tab). Command prompts also support `first`/`last` (Home/End) and
`complete` (Tab). Path prompts support `complete` (Tab); program prompts
support `open-actions` (Tab). Home/End in other prompts move the text cursor.
Plain characters and literal control characters cannot be assigned as prompt
actions. Defaults preserve their existing modifier rules; configured keys
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

Accepting a command from an explicitly configured key retains the same native
handoff eligibility as fresh Enter. Repeated or replayed input cannot acquire
that authority. Existing modified-Enter defaults retain their original limits.

The `file-picker` and `finder` owners support `accept`, `cancel`, `next`,
`previous`, `page-up`, `page-down`, `first`, `last`, `move-left`, `move-right`,
`move-start`, `move-end`, `backspace`, `delete-backward`, `delete`,
`delete-word-backward`, `delete-to-end`, `split-horizontal`, `split-vertical`,
and `toggle-preview`. `finder` also supports `toggle-mode` (plain Tab).
File-picker Tab aliases `next`; modified Tab in Finder remains unbound.
The existing Ctrl-s/Ctrl-v shortcuts open files horizontally/vertically;
splits remain unavailable for buffer and terminal rows.

These owners inherit leader cancellation only while the query is empty.
Once text exists, the leader resumes its configured ordinary meaning. Replacing
or removing `cancel` removes the inherited leader shortcut too. Literal filter
characters remain structural input. Picker `delete-backward` (Ctrl-h) retains
its existing ranking-publication behavior; `backspace` defers held ranking
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

Each list replaces all gestures for that action; `[]` disables the action's
pointer bindings. Omitted actions retain their defaults, including with
`keys.defaults: false`. Modifiers match exactly. Pane gestures currently accept
`LeftDrag`, with optional `Ctrl-`, `Alt-`, `Shift-`, `Super-`, `Hyper-`, or
`Meta-` modifiers. Shared pane borders retain resizing priority. The action is
captured when the button is pressed and lasts through release, even when the
modifiers or configuration change. Conflicting or invalid pointer bindings
retain both default pane gestures and produce a configuration diagnostic.
Contextual help shows the effective gestures. Configuration fragments merge
`pointer` by action; a later action list replaces the earlier list.

`Ctrl-Alt-LeftDrag` avoids terminals that reserve Shift for native selection.
The terminal or window manager still needs to deliver the configured gesture.

`keys.bind` has one map per mode: `normal`, `select`, `insert`, and
`replace`. Each maps a key sequence to one of four values:

| Value | Meaning | Example |
| --- | --- | --- |
| action name | Run one action. | `X: extend-line-above` |
| list | Run several actions in order. | `F6: [select-all, yank]` |
| `{ command, argument }` | Run a command with an argument. | `F7: { command: pipe, argument: sort }` |
| `null` | Remove the binding in this mode. | `F8: null` |

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
      F7: { command: pipe, argument: sort }
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

**Sort the selected lines:**

```yaml
keys:
  bind:
    normal:
      Alt-s: { command: pipe, argument: sort }
    select:
      Alt-s: { command: pipe, argument: sort }
```

**Switch to a particular theme** (`theme` is the palette name of
`open-theme-settings`):

```yaml
keys:
  bind:
    normal:
      F10: { command: theme, argument: mocha }
```

**Open a file:**

```yaml
keys:
  bind:
    normal:
      F11: { command: open, argument: README.md }
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
      F2: { command: help, argument: key-actions }
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
  modes that `bind` could assign, including the whole leader and window
  menus, and the keys that only answer with a message about a command Runyte
  does differently or does not have yet. That includes Insert mode's editing
  keys: printable characters still type, but `Enter`, `Backspace`, `Delete`,
  `Tab`, the arrows, `Home`, `End`, and the page keys do nothing until `bind`
  assigns them.
- **What stays:** keys that are not yours to assign. Special buffers such as
  the explorer, Git views, and help keep their own keys; so do lists, prompts,
  and terminals, which go on answering to the built-in Insert keys, such as
  `Ctrl-C` to copy and the window menu, as they do whenever `bind` replaces
  one. `Tab`, which runs a code action, and the count digits stay
  too, because `bind` cannot use them.
- **Two exits stay too, unless `bind` reassigns them.** `Ctrl-\` (`Ctrl-4` on
  legacy terminals) still returns to Normal mode, since inside a terminal
  `Esc` belongs to the program and `Ctrl-\` is the only way out of its input.
  With `editor.fast_pane_keys` on, `Ctrl-h`, `Ctrl-j`, `Ctrl-k`, and `Ctrl-l`
  keep moving between panes.
- **`rebind` is ignored**, with a message for each entry, since there is no
  built-in binding left to move. `leader` and `window` still set what `Leader`
  and `Window` mean in `bind`, and may be keys that only a built-in binding
  used.
- **A way out is required.** The keyboard must reach `open-command-palette`
  in Normal mode and `enter-normal-mode` in Select, Insert, and Replace modes,
  directly or as one of a sequence of actions. `Ctrl-\` covers the second
  unless you reassign both of its spellings. Without them you could not leave
  a mode or reach the configuration to fix it, so Runyte reports what is
  missing and keeps the built-in keys underneath your `bind` instead.

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
  reservations too; unreachable remappings are rejected and retain their
  working defaults.
- **Scoped input wins.** Buffer-specific keys, prompts, lists, native
  confirmations, and terminal input keep handling their own keys.

### Action sequences

A list runs its actions directly, one after another. It never types keys or
triggers other bindings.

**Order.** Each action in the reference is marked `continue` or `last`.

- A `continue` action can be followed by another action.
- A `last` action must end the list. These are actions that open a prompt,
  wait for a character, change mode, or start background work.
- So a list can end with `find-next-char`, which then waits for a character,
  or with `enter-insert-mode`, which then accepts typing.
- Nothing continues automatically after a prompt or background operation.

**Failure.** An error or unavailable action stops the list.

- Actions that already ran stay applied.
- A list is not rolled back. A saved file stays saved.

**Counts.** A single action keeps its usual count behavior. A count before a
multi-action binding is rejected before anything runs.

**Undo.**

- A list of selection-only actions is one selection-history step.
- Text undo follows each action's own checkpoints and Insert-mode grouping. A
  list is not one undoable change.

Paste stays literal, and macros record the physical keys you press, as before.

### Limits and errors

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
removed or overwrote is no longer advertised; teaching text shows another
binding for the action, or says it is unbound.

### Bindable action reference

This is the list `:help key-actions` shows, grouped by topic.

- **Modes:** N normal, S select, I insert, R replace.
- **Position:** `continue` can be followed by another action in a list;
  `last` must end it. See [Action sequences](custom-key-bindings.md#action-sequences).

#### Mode and page actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `enter-normal-mode` | N S I R | last | Return to normal mode |
| `open-command-palette` | N S | last | Open the command palette |
| `enter-insert-mode` | N S | last | Insert before the selection |
| `enter-replace-mode` | N S | last | Replace text from each selection head |
| `append-after` | N S | last | Insert after the selection |
| `insert-line-start` | N S | last | Insert at line start |
| `insert-line-end` | N S | last | Insert at line end |
| `open-line-below` | N S | last | Open a line below |
| `open-line-above` | N S | last | Open a line above |
| `enter-select-mode` | N S | last | Toggle select mode |
| `show-help` | N S | last | Open general or contextual Runyte help |
| `open-settings` | N S | last | Open editor settings |
| `open-theme-settings` | N S | last | Choose and save the editor theme |

#### Movement actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `move-left` | N S I R | continue | Move left |
| `move-right` | N S I R | continue | Move right |
| `move-up` | N S I R | continue | Move up |
| `move-down` | N S I R | continue | Move down |
| `move-line-start` | N S I R | continue | Move to line start |
| `move-line-end` | N S I R | continue | Move to line end |
| `move-first-non-whitespace` | N S | continue | Move to first non-whitespace character |
| `move-file-start` | N S | continue | Move to file start |
| `move-file-end` | N S | continue | Move to file end |
| `move-word-forward` | N S | continue | Move to next word start |
| `move-word-backward` | N S | continue | Move to previous word start |
| `move-word-end` | N S | continue | Move to next word end |
| `move-long-word-forward` | N S | continue | Move to next WORD start |
| `move-long-word-backward` | N S | continue | Move to previous WORD start |
| `move-long-word-end` | N S | continue | Move to next WORD end |
| `goto-next-paragraph` | N S | continue | Go to the next paragraph |
| `goto-previous-paragraph` | N S | continue | Go to the previous paragraph |
| `find-next-char` | N S | last | Find next character |
| `find-previous-char` | N S | last | Find previous character |
| `find-till-next-char` | N S | last | Find before next character |
| `find-till-previous-char` | N S | last | Find after previous character |
| `page-up` | N S I R | continue | Move one page up |
| `page-down` | N S I R | continue | Move one page down |
| `half-page-up` | N S | continue | Move half a page up |
| `half-page-down` | N S | continue | Move half a page down |
| `goto-window-top` | N S | continue | Move to the top of the view |
| `goto-window-center` | N S | continue | Move to the center of the view |
| `goto-window-bottom` | N S | continue | Move to the bottom of the view |
| `goto-word` | N S | last | Label visible words by proximity and jump to one |
| `match-bracket` | N S | last | Go to the matching syntax bracket |
| `jump-backward` | N S | last | Jump to the previous position |
| `jump-forward` | N S | last | Jump to the next position |
| `jump-backward-buffer` | N S | last | Jump to the previous buffer or terminal surface |
| `jump-forward-buffer` | N S | last | Jump to the next buffer or terminal surface |

#### Editing actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `replace-char` | N S | last | Replace selection with a character |
| `surround-add` | N S | last | Surround selections with a pair |
| `surround-replace` | N S | last | Replace the surrounding pair |
| `surround-delete` | N S | last | Delete the surrounding pair |
| `toggle-case` | N S | continue | Switch case of the selection |
| `undo` | N S | continue | Undo the last change |
| `redo` | N S | continue | Redo the last change |
| `yank` | N S | continue | Yank the selection or character |
| `yank-line` | N S | continue | Yank the lines the selection touches |
| `paste-after` | N S | continue | Replace the selection, or paste after the caret |
| `paste-before` | N S | continue | Paste before the selection |
| `indent` | N S | continue | Indent the selected lines |
| `unindent` | N S | continue | Unindent the selected lines |
| `toggle-comments` | N S I R | continue | Comment or uncomment the selected lines |
| `delete-selection` | N S | continue | Delete the selection or character |
| `change-selection` | N S | last | Change the selection or character |
| `rotate-selection-contents-forward` | N S | last | Rotate selected text forward |
| `rotate-selection-contents-backward` | N S | last | Rotate selected text backward |
| `trim-trailing-whitespace` | N S | last | Delete trailing whitespace from every selected line |
| `hard-wrap` | N S | last | Hard-wrap the selection |
| `reflow` | N S | last | Reflow paragraphs in the selection |
| `join-selections` | N S | last | Join the selected lines with a typed delimiter |
| `join-lines` | N S | last | Join the selected lines, or the line below, with a space |
| `format-table` | N S | last | Align the columns of the selected table |

#### Insert and Replace mode actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `delete-word-backward` | I R | continue | Delete the previous word |
| `delete-word-forward` | I R | continue | Delete the next word |
| `delete-to-line-start` | I R | continue | Delete to the start of the line |
| `delete-to-line-end` | I R | continue | Delete to the end of the line |
| `delete-char-backward` | I R | continue | Delete the previous character |
| `delete-char-forward` | I R | continue | Delete the next character |
| `insert-newline` | I R | continue | Insert a new line |
| `insert-indent` | I R | continue | Insert configured indentation |
| `insert-other-indent` | I R | continue | Insert the other indentation style |

#### Selection actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `selection-undo` | N S | continue | Undo the last selection change |
| `selection-redo` | N S | continue | Redo the last selection change |
| `select-line` | N S | continue | Select the current line, then extend downward |
| `extend-line-above` | N S | continue | Extend whole lines above |
| `extend-line-below` | N S | continue | Extend whole lines below |
| `select-line-up` | N S | continue | Select the current line, then extend upward |
| `select-all` | N S | continue | Select all text |
| `collapse-selection` | N S | continue | Collapse selection to the cursor |
| `flip-selection` | N S | continue | Flip the selection anchor and cursor |
| `split-selection-at-line-ends` | N S | continue | Place a cursor at the end of every selected line |
| `split-selection-at-line-starts` | N S | continue | Place a cursor at the start of every selected line |
| `keep-primary-selection` | N S | continue | Drop every selection except the primary |
| `remove-primary-selection` | N S | continue | Drop the primary selection |
| `copy-selection-down` | N S | continue | Add a cursor on the line below |
| `copy-selection-up` | N S | continue | Add a cursor on the line above |
| `copy-selection-down-padded` | N S | continue | Add a cursor on the line below, padding it when needed |
| `copy-selection-up-padded` | N S | continue | Add a cursor on the line above, padding it when needed |
| `rotate-selection-forward` | N S | continue | Make the next selection primary |
| `rotate-selection-backward` | N S | continue | Make the previous selection primary |
| `keep-matching-selections` | N S | last | Keep selections matching a regular expression |
| `remove-matching-selections` | N S | last | Remove selections matching a regular expression |
| `align-selections` | N S | continue | Pad with spaces so every cursor shares the rightmost column |
| `trim-selections` | N S | continue | Trim whitespace from every selection |

#### Text object actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `select-around-parentheses` | N S | last | Select around parentheses |
| `select-inside-parentheses` | N S | last | Select inside parentheses |
| `select-around-square-brackets` | N S | last | Select around square brackets |
| `select-inside-square-brackets` | N S | last | Select inside square brackets |
| `select-around-braces` | N S | last | Select around braces |
| `select-inside-braces` | N S | last | Select inside braces |
| `select-around-angle-brackets` | N S | last | Select around angle brackets |
| `select-inside-angle-brackets` | N S | last | Select inside angle brackets |
| `select-around-double-quotes` | N S | last | Select around double quotes |
| `select-inside-double-quotes` | N S | last | Select inside double quotes |
| `select-around-single-quotes` | N S | last | Select around single quotes |
| `select-inside-single-quotes` | N S | last | Select inside single quotes |
| `select-around-backticks` | N S | last | Select around backticks |
| `select-inside-backticks` | N S | last | Select inside backticks |
| `select-inside-word` | N S | last | Select the word under the cursor |
| `select-around-word` | N S | last | Select the word under the cursor and the space beside it |
| `select-inside-long-word` | N S | last | Select the WORD under the cursor |
| `select-around-long-word` | N S | last | Select the WORD under the cursor and the space beside it |
| `select-inside-paragraph` | N S | last | Select the paragraph's lines |
| `select-around-paragraph` | N S | last | Select the paragraph's lines and the blank lines beside them |
| `select-around-closest-delimiter` | N S | last | Select around the closest delimiter pair |
| `select-inside-closest-delimiter` | N S | last | Select inside the closest delimiter pair |

#### Syntax tree actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `expand-syntax-selection` | N S | last | Expand to the enclosing syntax node |
| `shrink-syntax-selection` | N S | last | Shrink the syntax selection |
| `select-syntax-parent` | N S | last | Select the enclosing syntax node |
| `select-syntax-child` | N S | last | Select the first child syntax node |
| `select-previous-syntax-sibling` | N S | last | Select the previous sibling syntax node |
| `select-next-syntax-sibling` | N S | last | Select the next sibling syntax node |
| `select-syntax-function` | N S | last | Select the enclosing function |
| `select-inside-syntax-function` | N S | last | Select inside the enclosing function |
| `select-syntax-class` | N S | last | Select the enclosing class-like item |
| `select-inside-syntax-class` | N S | last | Select inside the enclosing class-like item |
| `select-syntax-parameter` | N S | last | Select the enclosing parameter |
| `select-inside-syntax-parameter` | N S | last | Select inside the enclosing parameter |
| `goto-previous-syntax-function` | N S | last | Go to the previous function |
| `goto-next-syntax-function` | N S | last | Go to the next function |
| `goto-previous-syntax-class` | N S | last | Go to the previous class-like item |
| `goto-next-syntax-class` | N S | last | Go to the next class-like item |
| `goto-previous-syntax-parameter` | N S | last | Go to the previous parameter |
| `goto-next-syntax-parameter` | N S | last | Go to the next parameter |
| `document-outline` | N S | last | Open the document outline |
| `toggle-syntax-fold` | N S | last | Toggle the syntax fold at the cursor |
| `fold-all-syntax` | N S | last | Fold every syntax region |
| `unfold-all-syntax` | N S | last | Unfold every syntax region |

#### Search actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `search` | N S | last | Search for text, ignoring case |
| `search-regex` | N S | last | Search with a regular expression |
| `search-next` | N S | last | Select only the next search match |
| `search-previous` | N S | last | Select only the previous search match |
| `search-selection` | N S | last | Select every match of the selection or word |
| `global-search` | N S | last | Search the workspace, ignoring case |
| `global-search-regex` | N S | last | Search the workspace with a regular expression |

#### View actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `align-view-center` | N S | last | Center the cursor line in the view |
| `align-view-top` | N S | last | Align the cursor line at the top |
| `align-view-bottom` | N S | last | Align the cursor line at the bottom |
| `align-view-middle` | N S | last | Center the cursor column in the view |
| `scroll-view-down` | N S | last | Scroll the view down |
| `scroll-view-up` | N S | last | Scroll the view up |
| `toggle-soft-wrap` | N S | last | Toggle soft wrapping |
| `toggle-whitespace` | N S | last | Toggle whitespace markers |
| `toggle-zen` | N S | last | Toggle the centred, maximized writing view |
| `toggle-fullscreen` | N S | last | Toggle the active pane across the whole editor area |

#### File, buffer, and destination actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `save` | N S I R | last | Write the active buffer |
| `new-buffer` | N S | last | Open a new scratch buffer |
| `goto-file` | N S | last | Open the file or web link under the cursor |
| `open-explorer` | N S | last | Open file explorer in the active buffer's directory |
| `toggle-directory-tree` | N S | last | Show or hide the directory tree |
| `focus-directory-tree` | N S | last | Reveal the active file in the directory tree |
| `open-working-directory-explorer` | N S | last | Open file explorer in the working directory |
| `git-conflicts` | N S | last | Open unresolved Git conflicts |
| `open-file-picker` | N S | last | Open the finder over the project's files, buffers, and terminals |
| `open-all-files-picker` | N S | last | Open the finder over the project, including files Git ignores |
| `open-path-file-picker` | N S | last | Open the finder in a chosen path, including files Git ignores |
| `open-buffer-picker` | N S | last | Open the buffer picker |
| `open-navigator` | N S I R | last | Navigate open buffers and terminals |
| `previous-destination` | N S I R | last | Return to the previous destination in this pane |

#### Pane actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `split-vertical` | N S | last | Create a side-by-side split |
| `split-horizontal` | N S | last | Create a stacked split |
| `focus-window-left` | N S I R | last | Focus the pane to the left |
| `focus-window-down` | N S I R | last | Focus the pane below |
| `focus-window-up` | N S I R | last | Focus the pane above |
| `focus-window-right` | N S I R | last | Focus the pane to the right |
| `next-window` | N S I R | last | Focus the next pane |
| `swap-window` | N S I R | last | Swap this pane's content with the previously focused pane |
| `move-window-left` | N S I R | last | Move pane to the left edge, full height |
| `move-window-down` | N S I R | last | Move pane to the bottom edge, full width |
| `move-window-up` | N S I R | last | Move pane to the top edge, full width |
| `move-window-right` | N S I R | last | Move pane to the right edge, full height |
| `close-window` | N S | last | Close the active pane |
| `only-window` | N S | last | Close every pane except the active pane |
| `equalize-windows` | N S | last | Equalize pane widths, then pane heights within each column |

#### Terminal actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `open-terminal` | N S | last | Start a terminal in this pane |
| `open-terminal-list` | N S | last | Show the running terminals |
| `rename-terminal` | N S | last | Name the active terminal session |
| `leave-terminal` | N S | last | Show this pane's buffer again |
| `copy-terminal-output` | N S | last | Open this terminal's output as a buffer |
| `send-to-terminal` | N S | last | Send the selection to a terminal |

#### Persistent session actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `previous-session` | N S I R | last | Return to the previously visited persistent session |
| `session-1` | N S | last | Attach to persistent session 1 |
| `session-2` | N S | last | Attach to persistent session 2 |
| `session-3` | N S | last | Attach to persistent session 3 |
| `session-4` | N S | last | Attach to persistent session 4 |
| `session-5` | N S | last | Attach to persistent session 5 |
| `session-6` | N S | last | Attach to persistent session 6 |
| `session-7` | N S | last | Attach to persistent session 7 |
| `session-8` | N S | last | Attach to persistent session 8 |
| `session-9` | N S | last | Attach to persistent session 9 |
| `next-running-session` | N S I R | last | Visit the next running persistent session |
| `previous-running-session` | N S I R | last | Visit the previous running persistent session |

#### Language server actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `goto-definition` | N S | last | Go to definition |
| `goto-declaration` | N S | last | Go to declaration |
| `goto-type-definition` | N S | last | Go to type definition |
| `goto-references` | N S | last | Go to references |
| `goto-implementation` | N S | last | Go to implementation |
| `show-documentation` | N S | last | Show documentation |
| `document-symbols` | N S | last | Open document symbols |
| `workspace-symbols` | N S | last | Open workspace symbols |
| `diagnostics` | N S | last | Open diagnostics |
| `trigger-completion` | N S I R | last | Ask the language server for completions |
| `rename-symbol` | N S | last | Rename symbol |
| `code-action` | N S | last | Apply a code action |

#### Clipboard, register, and macro actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `clipboard-yank` | N S | last | Yank to the system clipboard |
| `clipboard-paste-after` | N S | last | Paste from the system clipboard |
| `clipboard-paste-before` | N S | last | Paste before from the system clipboard |
| `clipboard-paste` | N S I R | last | Paste an image or text from the clipboard |
| `select-register` | N S | last | Select a register |
| `record-macro` | N S | last | Record a macro named by the next key |
| `record-default-macro` | N S | last | Record the default macro, or stop recording |
| `replay-macro` | N S | last | Replay the macro named by the next key |
| `replay-default-macro` | N S | last | Replay the default macro |
| `list-macros` | N S | last | List the recorded macros |

#### Colon commands

These run in Normal and Select modes and are always `last`. Pass an argument
with `{ command: name, argument: text }`.

| Command | Usage | Description |
| --- | --- | --- |
| `cd` | `cd <path>` | Change the editor working directory |
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
| `git-log` | `git-log` | Open the Git log, or refresh it from its first page |
| `git-network` | `git-network` | Open or refresh the cached commit network |
| `git-network-head` | `git-network-head` | Restrict the commit network to HEAD |
| `git-network-all` | `git-network-all` | Show all cached refs and HEAD |
| `git-network-choose-ref` | `git-network-choose-ref` | Choose a captured exact ref |
| `git-network-ref` | `git-network-ref <full-ref>` | Restrict the commit network to an exact full ref |
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
| `mcp` | `mcp [identity]` | Review workspace agent permissions |
| `agent-channel` | `agent-channel` | Show this workspace's agent channel |
| `lsp-restart` | `lsp-restart [language]` | Restart stopped language servers |
| `lsp-status` | `lsp-status` | Report language server state |
| `open` | `open <path>` | Open a file or directory in the active pane |
| `detach` | `detach` | Detach the TUI from its persistent session |
| `quit` | `quit` | Close the active pane and its unique buffer, or quit from the last pane |
| `quit!` | `quit!` | Discard a unique buffer and close its pane, or force quit from the last pane |
| `quit-all` | `quit-all` | Quit if all buffers are saved and no terminal is running |
| `quit-all!` | `quit-all!` | Quit and discard unsaved buffers, but never terminate terminals |
| `quit-all!!` | `quit-all!!` | Discard unsaved buffers, terminate this workspace’s terminals and quit |
| `quit-here` | `quit-here` | Quit and hand the active directory to the shell wrapper |
| `quit-here!` | `quit-here!` | Discard changes, quit, and hand the active directory to the shell wrapper |
| `reload` | `reload` | Reload the active view |
| `resize-right` | `resize-right <+\|-> <cells>` | Grow or shrink the active pane at its right edge |
| `resize-left` | `resize-left <+\|-> <cells>` | Grow or shrink the active pane at its left edge |
| `resize-top` | `resize-top <+\|-> <cells>` | Grow or shrink the active pane at its top edge |
| `resize-bottom` | `resize-bottom <+\|-> <cells>` | Grow or shrink the active pane at its bottom edge |
| `write-quit` | `write-quit` | Write the active buffer, then close its pane or quit if it is the last pane |
| `write-buffer-close` | `write-buffer-close` | Write and close the active buffer without changing the pane layout |

<!-- navigation -->
[Contents](README.md) · [Previous: Configuration](configuration.md) · [Next: Themes](themes.md)

`ui.macro-replay.cancel` stops cooperative macro replay. It defaults to Escape
with any modifiers and exact Ctrl-c. While replay is running, other physical
input remains ignored. Replacing this action removes both default aliases;
`[]` disables keyboard cancellation. The leader does not add a cancellation
alias. The progress message shows the effective keys.

`ui.hover.accept` opens the full documentation buffer from a long hover popup
(default: plain Enter). Short hovers and other keys retain dismissal followed
by normal input handling. Command prompts, completion and pending key sequences
retain precedence. Character operands and plain count digits remain structural.
Hover hints show only reachable aliases. The hover owner adds no leader alias. `[]` removes the expansion shortcut.

`ui.character-input.cancel` cancels a pending character operand, such as the
character after `f` or either operand of `m r`. It defaults to Escape with any
modifiers. Character keys, including modified characters, remain operands and
cannot be assigned to cancellation. Named keys and function keys are available.
Other non-character keys retain their existing “expected a character” result;
removing cancellation does not make them valid operands. The owner adds no
leader alias.

`ui.grammar-prefix` supports `cancel` (Escape) and `back` (Backspace) while a
Normal/Select key sequence is pending. Cancellation clears its count; backtracking
keeps it. Both default keys accept all modifiers. The same key can still start a
sequence when nothing is pending. Insert/Replace retain their existing prefix
fallback behavior. Character operands use their separate owner above.

`ui.key-hints` supports `next` and `previous`, defaulting to exact Ctrl-n/Ctrl-p
and plain Down/Up. Default arrows yield to registered continuations; explicitly
configured arrows own pending hint input. Window-prefix hints in Insert/Replace
also support configured scrolling. Both frontends show effective scroll keys.

Prefix controls and hint scrolling are validated together against the final
binding map. Null removals can release conflicting bindings; released control
keys can then appear inside configured sequences. If the final map still has a
collision, both of these UI owners return to defaults and binding edits are
compiled again against that policy. Other UI owners keep their configuration.
A key cannot belong to both prefix control and hint scrolling. Merge reviews
retain their separate scoped controls and do not pass through hint scrolling.

`ui.context-launcher.open-actions` replaces the Normal/Select action-menu shortcut
(default: plain Tab). Pending key sequences, character operands and the directory
tree retain precedence; plain count digits remain structural. Replacing or
removing the shortcut releases Tab for `keys.bind` and scoped assignments.
Menu mnemonic letters retain their existing structural role.

When no native action is available, the launcher keeps its existing fallback:
without a ready language server it reports that no actions are available; with
one it passes the actual pressed key to the editor keymap, retaining the count.
For a remapped launcher, assign the desired editor fallback to that same key.
It does not synthesize Tab. Help prose and action tables show the effective key.
