## Custom key bindings

The `keys` section configures keyboard actions and pane gestures:

| Key | What it does |
| --- | --- |
| `keys.leader` | Replaces `Space` as the leader prefix. |
| `keys.window` | Replaces `Ctrl-w` as the window prefix. |
| `keys.rebind` | Moves bindings Runyte already ships to other keys. |
| `keys.bind` | Assigns named actions to keys, per mode. Also removes bindings. |
| `keys.defaults` | `false` starts `bind` from an empty keyboard instead of Runyte's. See [Without the built-in keys](custom-key-bindings.md#without-the-built-in-keys). |
| `keys.pointer` | Replaces editor, pane, media, and preview pointer gestures. |
| `keys.ui` | Replaces actions in prompts, menus, overlays, confirmations, and native controls. |
| `keys.scopes` | Assigns or removes actions in individual editor views and result lists. |

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
their bindings. Suppression also hides inherited bindings. It does not reveal
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
and any following sequence. An explicit persistent-session action also claims
its key outside a persistent session and tells you when it is unavailable. A
`null` removes only the exact binding and its legacy capture policy. Longer
surviving bindings still claim their prefix. Removing `Ctrl-w` alone therefore
does not remove the window namespace. Unmodified Escape still reaches the child
unless explicitly assigned. Global custom text-editing bindings remain excluded
from Terminal Insert.

Use `global.list` for ordinary result-list bindings and `session-manager.list`
for overrides in the session manager. List bindings contain one key. List
navigation actions are `list-next`, `list-previous`, `list-page-down`,
`list-page-up`, `list-first`, `list-last`, `list-toggle-preview`,
`list-clear-filter`, `list-close-target`, and `list-close`. Use `list-accept` to activate a row and
`list-backspace` to delete the last filter character. `list-open-actions` opens
a row's actions. When no action menu applies, it cycles tags or moves to the
next row. These three actions do nothing in reports. Their inherited Enter, Tab
and Backspace bindings accept every modifier combination, preserving the
original behavior. Scoped overrides replace or remove only the exact spelling.
For example, `Enter: null` leaves `Shift-Enter` available. Empty-filter leader
input follows the effective Escape action, including its removal. Nested action
menus own their input separately. Numbered session choices remain structural
input.

`Delete` (`Fn-Delete` on a compact Mac keyboard) invokes `list-close-target`
in buffer, terminal and persistent-session lists. It closes the selected resource,
whereas `list-close` dismisses the list. Backspace only edits the filter, including
when it is empty. `list-clear-filter` has no default key; bind it if desired:

```yaml
keys:
  scopes:
    global:
      list:
        Delete: null
        F8: list-close-target
        F9: list-clear-filter
```

Close confirmations start on Cancel. Their selection, acceptance and cancellation
keys are configurable under `keys.ui.list-close` (`next`, `previous`, `accept`,
`cancel`). Repeated input cannot accept a close confirmation.

`:help key-actions` includes the scoped action names and their modes.
Actions must be supported by the selected view and mode. Scoped assignments
currently take one action name, without arguments or action lists. Editor
sequences may contain up to eight keys, including `Leader` and `Window`.
Context-action Tab, overlay leader dismissal,
and merge acknowledgment editing still belong to their existing input owners;
assignments or removals that those handlers would bypass are rejected.

Assignments apply simultaneously. Invalid scoped configuration rolls back the
**whole `scopes` section**, while `bind`, `ui`, and `pointer` remain
independent. A child override wins over inherited parents. If both Markdown and
conflict-file scopes override the same key, `git-conflict-markdown` must decide
explicitly, including by assigning `null`. Configuration fragments merge by
scope, mode, and key. Existing scoped defaults remain with
`keys.defaults: false`. That option still controls only global editor defaults.

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

Merge-review controls use `keys.scopes.git-merge-review`, `git-merge-detail`,
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

`yank` leaves the selection and mode alone, so the buffer stays selected
afterwards. Use `yank-collapse-normal-mode`, what `y` runs, to end on a caret
in Normal mode instead.

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

**Order.** Each action in the reference is marked `continue` or `last`.

- A `continue` action can be followed by another action.
- A `last` action must end the list. These are actions that open a prompt,
  wait for a character, change mode, or start background work.
- So a list can end with `find-next-char`, which then waits for a character,
  or with `enter-insert-mode`, which then accepts typing.
- Nothing continues automatically after a prompt or background operation.

**Failure.** An error or unavailable action stops the list. So does
`search-next` or `search-previous` finding no match, so the actions after it
never run against a selection the search did not choose.

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
removed or overwrote is no longer advertised. Teaching text shows another
binding for the action, or says it is unbound.

### Discovering actions and bindings

The provisional commands `:describe-key`, `:action-browser`, and
`:copy-key-binding` have no default keys while their names are under review.
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

### Bindable action reference

Regenerate this block with `cargo run --example action_reference`.

<a id="mode-and-page-actions"></a>
<a id="movement-actions"></a>
<a id="editing-actions"></a>
<a id="insert-and-replace-mode-actions"></a>
<a id="selection-actions"></a>
<a id="text-object-actions"></a>
<a id="syntax-tree-actions"></a>
<a id="search-actions"></a>
<a id="view-actions"></a>
<a id="file-buffer-and-destination-actions"></a>
<a id="pane-actions"></a>
<a id="terminal-actions"></a>
<a id="persistent-session-actions"></a>
<a id="language-server-actions"></a>
<a id="clipboard-register-and-macro-actions"></a>
<a id="colon-commands"></a>
<!-- action-reference:start -->

Use these names in `keys.bind`. Modes are independent. Actions marked `last`
must end a sequence; `continue` actions may precede another action. Counts
on multi-action bindings are rejected. Plugin bindings use `plugins[].bindings`.

| Action | Modes | Sequence position | Description |
| --- | --- | --- | --- |
| `enter-normal-mode` | normal, select, insert, replace | last | Return to normal mode |
| `open-command-palette` | normal, select | last | Open the command palette |
| `move-left` | normal, select, insert, replace | continue | Move left |
| `move-right` | normal, select, insert, replace | continue | Move right |
| `move-up` | normal, select, insert, replace | continue | Move up |
| `move-down` | normal, select, insert, replace | continue | Move down |
| `move-line-start` | normal, select, insert, replace | continue | Move to line start |
| `move-line-end` | normal, select, insert, replace | continue | Move to line end |
| `move-first-non-whitespace` | normal, select | continue | Move to first non-whitespace character |
| `move-file-start` | normal, select | continue | Move to file start |
| `move-file-end` | normal, select | continue | Move to file end |
| `move-word-forward` | normal, select | continue | Move to next word start |
| `move-word-backward` | normal, select | continue | Move to previous word start |
| `move-word-end` | normal, select | continue | Move to next word end |
| `move-long-word-forward` | normal, select | continue | Move to next WORD start |
| `move-long-word-backward` | normal, select | continue | Move to previous WORD start |
| `move-long-word-end` | normal, select | continue | Move to next WORD end |
| `goto-next-paragraph` | normal, select | continue | Go to the next paragraph |
| `goto-previous-paragraph` | normal, select | continue | Go to the previous paragraph |
| `find-next-char` | normal, select | last | Find next character |
| `find-previous-char` | normal, select | last | Find previous character |
| `find-till-next-char` | normal, select | last | Find before next character |
| `find-till-previous-char` | normal, select | last | Find after previous character |
| `page-up` | normal, select, insert, replace | continue | Move one page up |
| `page-down` | normal, select, insert, replace | continue | Move one page down |
| `half-page-up` | normal, select | continue | Move half a page up |
| `half-page-down` | normal, select | continue | Move half a page down |
| `enter-insert-mode` | normal, select | last | Insert before the selection |
| `enter-replace-mode` | normal, select | last | Replace text from each selection head |
| `append-after` | normal, select | last | Insert after the selection |
| `insert-line-start` | normal, select | last | Insert at line start |
| `insert-line-end` | normal, select | last | Insert at line end |
| `open-line-below` | normal, select | last | Open a line below |
| `open-line-above` | normal, select | last | Open a line above |
| `replace-char` | normal, select | last | Replace selection with a character |
| `surround-add` | normal, select | last | Surround selections with a pair |
| `surround-replace` | normal, select | last | Replace the surrounding pair |
| `surround-delete` | normal, select | last | Delete the surrounding pair |
| `toggle-case` | normal, select | continue | Switch case of the selection |
| `selection-undo` | normal, select | continue | Undo the last selection change |
| `selection-redo` | normal, select | continue | Redo the last selection change |
| `undo` | normal, select | continue | Undo the last change |
| `redo` | normal, select | continue | Redo the last change |
| `yank` | normal, select | continue | Yank the selection or character |
| `yank-line` | normal, select | continue | Yank the lines the selection touches |
| `yank-collapse-normal-mode` | normal, select | continue | Yank, then leave a caret in normal mode |
| `yank-line-collapse-normal-mode` | normal, select | continue | Yank lines, then leave a normal-mode caret |
| `paste-after-or-replace` | normal, select | continue | Replace the selection, or paste after the caret |
| `paste-after` | normal, select | continue | Paste after the selection or caret |
| `replace-with-register` | normal, select | continue | Replace the selection or character |
| `paste-before` | normal, select | continue | Paste before the selection |
| `indent` | normal, select | continue | Indent the selected lines |
| `unindent` | normal, select | continue | Unindent the selected lines |
| `toggle-comments` | normal, select, insert, replace | continue | Comment or uncomment the selected lines |
| `yank-delete-selection` | normal, select | continue | Yank, then delete the selection or character |
| `yank-change-selection` | normal, select | last | Yank, then change the selection or character |
| `delete-selection` | normal, select | continue | Delete the selection or character without yanking |
| `change-selection` | normal, select | last | Change the selection or character without yanking |
| `enter-select-mode` | normal, select | last | Toggle select mode |
| `select-line` | normal, select | continue | Select the current line, then extend downward |
| `extend-line-above` | normal, select | continue | Extend whole lines above |
| `extend-line-below` | normal, select | continue | Extend whole lines below |
| `select-line-up` | normal, select | continue | Select the current line, then extend upward |
| `select-all` | normal, select | continue | Select all text |
| `collapse-selection` | normal, select | continue | Collapse selection to the cursor |
| `flip-selection` | normal, select | continue | Flip the selection anchor and cursor |
| `expand-syntax-selection` | normal, select | last | Expand to the enclosing syntax node |
| `shrink-syntax-selection` | normal, select | last | Shrink the syntax selection |
| `select-syntax-parent` | normal, select | last | Select the enclosing syntax node |
| `select-syntax-child` | normal, select | last | Select the first child syntax node |
| `select-previous-syntax-sibling` | normal, select | last | Select the previous sibling syntax node |
| `select-next-syntax-sibling` | normal, select | last | Select the next sibling syntax node |
| `select-syntax-function` | normal, select | last | Select the enclosing function |
| `select-inside-syntax-function` | normal, select | last | Select inside the enclosing function |
| `select-syntax-class` | normal, select | last | Select the enclosing class-like item |
| `select-inside-syntax-class` | normal, select | last | Select inside the enclosing class-like item |
| `select-syntax-parameter` | normal, select | last | Select the enclosing parameter |
| `select-inside-syntax-parameter` | normal, select | last | Select inside the enclosing parameter |
| `select-around-parentheses` | normal, select | last | Select around parentheses |
| `select-inside-parentheses` | normal, select | last | Select inside parentheses |
| `select-around-square-brackets` | normal, select | last | Select around square brackets |
| `select-inside-square-brackets` | normal, select | last | Select inside square brackets |
| `select-around-braces` | normal, select | last | Select around braces |
| `select-inside-braces` | normal, select | last | Select inside braces |
| `select-around-angle-brackets` | normal, select | last | Select around angle brackets |
| `select-inside-angle-brackets` | normal, select | last | Select inside angle brackets |
| `select-around-double-quotes` | normal, select | last | Select around double quotes |
| `select-inside-double-quotes` | normal, select | last | Select inside double quotes |
| `select-around-single-quotes` | normal, select | last | Select around single quotes |
| `select-inside-single-quotes` | normal, select | last | Select inside single quotes |
| `select-around-backticks` | normal, select | last | Select around backticks |
| `select-inside-backticks` | normal, select | last | Select inside backticks |
| `select-inside-word` | normal, select | last | Select the word under the cursor |
| `select-around-word` | normal, select | last | Select the word under the cursor and the space beside it |
| `select-inside-long-word` | normal, select | last | Select the WORD under the cursor |
| `select-around-long-word` | normal, select | last | Select the WORD under the cursor and the space beside it |
| `select-inside-paragraph` | normal, select | last | Select the paragraph's lines |
| `select-around-paragraph` | normal, select | last | Select the paragraph's lines and the blank lines beside them |
| `select-around-closest-delimiter` | normal, select | last | Select around the closest delimiter pair |
| `select-inside-closest-delimiter` | normal, select | last | Select inside the closest delimiter pair |
| `goto-previous-syntax-function` | normal, select | last | Go to the previous function |
| `goto-next-syntax-function` | normal, select | last | Go to the next function |
| `goto-previous-syntax-class` | normal, select | last | Go to the previous class-like item |
| `goto-next-syntax-class` | normal, select | last | Go to the next class-like item |
| `goto-previous-syntax-parameter` | normal, select | last | Go to the previous parameter |
| `goto-next-syntax-parameter` | normal, select | last | Go to the next parameter |
| `document-outline` | normal, select | last | Open the document outline |
| `toggle-syntax-fold` | normal, select | last | Toggle the syntax fold at the cursor |
| `fold-all-syntax` | normal, select | last | Fold every syntax region |
| `unfold-all-syntax` | normal, select | last | Unfold every syntax region |
| `split-selection-at-line-ends` | normal, select | continue | Place a cursor at the end of every selected line |
| `split-selection-at-line-starts` | normal, select | continue | Place a cursor at the start of every selected line |
| `keep-primary-selection` | normal, select | continue | Drop every selection except the primary |
| `remove-primary-selection` | normal, select | continue | Drop the primary selection |
| `copy-selection-down` | normal, select | continue | Add a cursor on the line below |
| `copy-selection-up` | normal, select | continue | Add a cursor on the line above |
| `copy-selection-down-padded` | normal, select | continue | Add a cursor on the line below, padding it when needed |
| `copy-selection-up-padded` | normal, select | continue | Add a cursor on the line above, padding it when needed |
| `rotate-selection-forward` | normal, select | continue | Make the next selection primary |
| `rotate-selection-backward` | normal, select | continue | Make the previous selection primary |
| `rotate-selection-contents-forward` | normal, select | last | Rotate selected text forward |
| `rotate-selection-contents-backward` | normal, select | last | Rotate selected text backward |
| `keep-matching-selections` | normal, select | last | Keep selections matching a regular expression |
| `remove-matching-selections` | normal, select | last | Remove selections matching a regular expression |
| `align-selections` | normal, select | continue | Pad with spaces so every cursor shares the rightmost column |
| `trim-selections` | normal, select | continue | Trim whitespace from every selection |
| `trim-trailing-whitespace` | normal, select | last | Delete trailing whitespace from every selected line |
| `hard-wrap` | normal, select | last | Hard-wrap the selection |
| `reflow` | normal, select | last | Reflow paragraphs in the selection |
| `join-selections` | normal, select | last | Join the selected lines with a typed delimiter |
| `join-lines` | normal, select | last | Join the selected lines, or the line below, with a space |
| `format-table` | normal, select | last | Align the columns of the selected table |
| `search` | normal, select | last | Search for text, ignoring case |
| `search-regex` | normal, select | last | Search with a regular expression |
| `search-next` | normal, select | continue | Select only the next search match |
| `search-previous` | normal, select | continue | Select only the previous search match |
| `search-selection` | normal, select | last | Select every match of the selection or word |
| `set-search-from-selection` | normal, select | continue | Use the selection or word as the search |
| `align-view-center` | normal, select | last | Center the cursor line in the view |
| `align-view-top` | normal, select | last | Align the cursor line at the top |
| `align-view-bottom` | normal, select | last | Align the cursor line at the bottom |
| `align-view-middle` | normal, select | last | Center the cursor column in the view |
| `scroll-view-down` | normal, select | last | Scroll the view down |
| `scroll-view-up` | normal, select | last | Scroll the view up |
| `goto-window-top` | normal, select | continue | Move to the top of the view |
| `goto-window-center` | normal, select | continue | Move to the center of the view |
| `goto-window-bottom` | normal, select | continue | Move to the bottom of the view |
| `goto-file` | normal, select | last | Open the file or web link under the cursor |
| `goto-word` | normal, select | last | Label visible words by proximity and jump to one |
| `toggle-soft-wrap` | normal, select | last | Toggle soft wrapping |
| `toggle-whitespace` | normal, select | last | Toggle whitespace markers |
| `toggle-zen` | normal, select | last | Toggle the centred, maximized writing view |
| `toggle-fullscreen` | normal, select | last | Toggle the active pane across the whole editor area |
| `open-explorer` | normal, select | last | Open file explorer in the active buffer's directory |
| `toggle-directory-tree` | normal, select | last | Show or hide the directory tree |
| `focus-directory-tree` | normal, select | last | Reveal the active file in the directory tree |
| `open-working-directory-explorer` | normal, select | last | Open file explorer in the working directory |
| `open-file-picker` | normal, select | last | Open the finder over the project's files, buffers, and terminals |
| `open-all-files-picker` | normal, select | last | Open the finder over the project, including files Git ignores |
| `open-path-file-picker` | normal, select | last | Open the finder in a chosen path, including files Git ignores |
| `git-conflicts` | normal, select | last | Open unresolved Git conflicts |
| `open-settings` | normal, select | last | Open editor settings |
| `open-theme-settings` | normal, select | last | Choose and save the editor theme |
| `split-vertical` | normal, select | last | Create a side-by-side split |
| `split-horizontal` | normal, select | last | Create a stacked split |
| `save` | normal, select, insert, replace | last | Write the active buffer |
| `show-help` | normal, select | last | Open general or contextual Runyte help |
| `focus-window-left` | normal, select, insert, replace | last | Focus the pane to the left |
| `focus-window-down` | normal, select, insert, replace | last | Focus the pane below |
| `focus-window-up` | normal, select, insert, replace | last | Focus the pane above |
| `focus-window-right` | normal, select, insert, replace | last | Focus the pane to the right |
| `next-window` | normal, select, insert, replace | last | Focus the next pane |
| `swap-window` | normal, select, insert, replace | last | Swap this pane's content with the previously focused pane |
| `move-window-left` | normal, select, insert, replace | last | Move pane to the left edge, full height |
| `move-window-down` | normal, select, insert, replace | last | Move pane to the bottom edge, full width |
| `move-window-up` | normal, select, insert, replace | last | Move pane to the top edge, full width |
| `move-window-right` | normal, select, insert, replace | last | Move pane to the right edge, full height |
| `close-window` | normal, select | last | Close the active pane |
| `only-window` | normal, select | last | Close every pane except the active pane |
| `equalize-windows` | normal, select | last | Equalize pane widths, then pane heights within each column |
| `open-terminal` | normal, select | last | Start a terminal in this pane |
| `open-terminal-list` | normal, select | last | Show the running terminals |
| `rename-terminal` | normal, select | last | Name the active terminal session |
| `leave-terminal` | normal, select | last | Show this pane's buffer again |
| `copy-terminal-output` | normal, select | last | Open this terminal's output as a buffer |
| `send-to-terminal` | normal, select | last | Send the selection to a terminal |
| `delete-word-backward` | insert, replace | continue | Delete the previous word |
| `delete-word-forward` | insert, replace | continue | Delete the next word |
| `delete-to-line-start` | insert, replace | continue | Delete to the start of the line |
| `delete-to-line-end` | insert, replace | continue | Delete to the end of the line |
| `delete-char-backward` | insert, replace | continue | Delete the previous character |
| `delete-char-forward` | insert, replace | continue | Delete the next character |
| `insert-newline` | insert, replace | continue | Insert a new line |
| `insert-indent` | insert, replace | continue | Insert configured indentation |
| `insert-other-indent` | insert, replace | continue | Insert the other indentation style |
| `goto-definition` | normal, select | last | Go to definition |
| `goto-declaration` | normal, select | last | Go to declaration |
| `goto-type-definition` | normal, select | last | Go to type definition |
| `goto-references` | normal, select | last | Go to references |
| `goto-implementation` | normal, select | last | Go to implementation |
| `new-buffer` | normal, select | last | Open a new scratch buffer |
| `open-navigator` | normal, select, insert, replace | last | Navigate open buffers and terminals |
| `previous-destination` | normal, select, insert, replace | last | Return to the previous destination in this pane |
| `previous-session` | normal, select, insert, replace | last | Return to the previously visited persistent session |
| `session-1` | normal, select | last | Attach to persistent session 1 |
| `session-2` | normal, select | last | Attach to persistent session 2 |
| `session-3` | normal, select | last | Attach to persistent session 3 |
| `session-4` | normal, select | last | Attach to persistent session 4 |
| `session-5` | normal, select | last | Attach to persistent session 5 |
| `session-6` | normal, select | last | Attach to persistent session 6 |
| `session-7` | normal, select | last | Attach to persistent session 7 |
| `session-8` | normal, select | last | Attach to persistent session 8 |
| `session-9` | normal, select | last | Attach to persistent session 9 |
| `next-running-session` | normal, select, insert, replace | last | Visit the next running persistent session |
| `previous-running-session` | normal, select, insert, replace | last | Visit the previous running persistent session |
| `open-buffer-picker` | normal, select | last | Open the buffer picker |
| `global-search` | normal, select | last | Search the workspace, ignoring case |
| `global-search-regex` | normal, select | last | Search the workspace with a regular expression |
| `show-documentation` | normal, select | last | Show documentation |
| `document-symbols` | normal, select | last | Open document symbols |
| `workspace-symbols` | normal, select | last | Open workspace symbols |
| `diagnostics` | normal, select | last | Open diagnostics |
| `trigger-completion` | normal, select, insert, replace | last | Ask the language server for completions |
| `jump-backward` | normal, select | last | Jump to the previous position |
| `jump-forward` | normal, select | last | Jump to the next position |
| `jump-backward-buffer` | normal, select | last | Jump to the previous buffer or terminal surface |
| `jump-forward-buffer` | normal, select | last | Jump to the next buffer or terminal surface |
| `rename-symbol` | normal, select | last | Rename symbol |
| `code-action` | normal, select | last | Apply a code action |
| `clipboard-paste-after` | normal, select | last | Paste from the system clipboard |
| `clipboard-paste-before` | normal, select | last | Paste before from the system clipboard |
| `clipboard-yank` | normal, select, insert, replace | last | Yank to the system clipboard |
| `clipboard-paste` | normal, select, insert, replace | last | Paste an image or text from the clipboard |
| `select-register` | normal, select | last | Select a register |
| `record-macro` | normal, select | last | Record a macro named by the next key |
| `record-default-macro` | normal, select | last | Record the default macro, or stop recording |
| `replay-macro` | normal, select | last | Replay the macro named by the next key |
| `replay-default-macro` | normal, select | last | Replay the default macro |
| `list-macros` | normal, select | last | List the recorded macros |
| `match-bracket` | normal, select | last | Go to the matching syntax bracket |

#### Colon commands

These accept the same arguments as the command palette, supplied separately
as `{ command: name, argument: text }`. They are available in Normal and
Select modes and must end a sequence. Required arguments must be supplied.

| Command | Usage | Description |
| --- | --- | --- |
| `describe-key` | `describe-key` | Inspect a key without executing it |
| `action-browser` | `action-browser` | Browse action descriptors and default keys |
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

Use one action name or null in `keys.scopes.<scope>.<mode>`. Ordinary editor
scopes also support the applicable global actions above. These additional
actions belong to the indicated owner; raw UI input still has its own scope.

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

- `choose-explorer-order`: normal, select
- `open-directory-entry`: normal, select
- `open-explorer-finder`: normal, select
- `open-explorer-session`: normal, select
- `open-explorer-system`: normal, select
- `open-explorer-workspace`: normal, select
- `open-parent-directory`: normal, select
- `open-terminal-directory-root`: normal, select
- `toggle-directory-details`: normal, select
- `toggle-hidden-files`: normal, select

##### directory-tree

- `directory-tree-close`: normal, select
- `directory-tree-delete`: normal, select
- `directory-tree-down`: normal, select
- `directory-tree-first`: normal, select
- `directory-tree-last`: normal, select
- `directory-tree-left`: normal, select
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
- `directory-tree-right`: normal, select
- `directory-tree-up`: normal, select
- `toggle-hidden-files`: normal, select

##### settings

- `activate-setting`: normal, select
- `override-setting-language`: normal, select
- `override-setting-pattern`: normal, select
- `remove-setting-override`: normal, select

##### git-status

- `git-merge-abort`: normal, select
- `git-merge-continue`: normal, select
- `open-changed-file`: normal, select
- `pull-branch`: normal, select
- `push-branch`: normal, select
- `stage-all-changed-files`: normal, select

##### git-branches

- `checkout-branch`: normal, select
- `create-branch`: normal, select
- `create-worktree`: normal, select
- `delete-branch`: normal, select
- `git-fetch-branch`: normal, select
- `git-merge-branch`: normal, select
- `pull-branch`: normal, select
- `push-branch`: normal, select

##### git-conflicts

- `git-conflict-file-current`: normal, select
- `git-conflict-file-other`: normal, select
- `git-conflict-resolve`: normal, select
- `git-conflict-resolve-literal-markers`: normal, select
- `git-conflict-sides`: normal, select
- `git-merge-abort`: normal, select
- `git-merge-continue`: normal, select
- `open-git-conflict`: normal, select

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
- `toggle-fullscreen`: insert
- `toggle-zen`: insert

##### git-worktrees

- `create-new-worktree`: normal, select
- `open-worktree`: normal, select
- `remove-worktree`: normal, select

##### git-log

- `next-git-log-page`: normal, select
- `open-git-commit`: normal, select
- `previous-git-log-page`: normal, select

##### git-network

- `next-git-network-page`: normal, select
- `open-git-commit`: normal, select
- `previous-git-network-page`: normal, select

##### git-blame

- `open-git-commit`: normal, select

##### workspace-search

- `open-workspace-search-result`: normal, select

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
- `open-session-destinations`: list
- `open-session-directory`: list


#### Action contracts

Generated from built-in descriptors. Effects include owned continuations and specialized views; they do not change the current sequence rules. Character operands and palette arguments are separate.

| Action | Effects | Count | Character operands | Palette argument | View differences |
| --- | --- | --- | --- | --- | --- |
| `enter-normal-mode` | [Mode, Selection] | Reject | Zero | none | terminal: Leave child input for live Normal mode; otherwise capture review and collapse its selection |
| `open-command-palette` | [Prompt] | Reject | Zero | none |  |
| `move-left` | [Selection, View] | Repeat | Zero | none | terminal: Move or extend the cursor in captured output; media: Pan horizontally, or extend the media selection in Select mode |
| `move-right` | [Selection, View] | Repeat | Zero | none | terminal: Move or extend the cursor in captured output; media: Pan horizontally, or extend the media selection in Select mode |
| `move-up` | [Selection, View] | Repeat | Zero | none | terminal: Move or extend the cursor in captured output; media: Pan vertically, change PDF page at fit, or extend the media selection |
| `move-down` | [Selection, View] | Repeat | Zero | none | terminal: Move or extend the cursor in captured output; media: Pan vertically, change PDF page at fit, or extend the media selection |
| `move-line-start` | [Selection] | Reject | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-line-end` | [Selection] | Reject | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-first-non-whitespace` | [Selection] | Reject | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-file-start` | [Selection, View] | Interpret | Zero | none | terminal: Move to the captured output boundary; media: Select a PDF page by address or move to its first or last page |
| `move-file-end` | [Selection, View] | Interpret | Zero | none | terminal: Move to the captured output boundary; media: Select a PDF page by address or move to its first or last page |
| `move-word-forward` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-word-backward` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-word-end` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-long-word-forward` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-long-word-backward` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `move-long-word-end` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `goto-next-paragraph` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `goto-previous-paragraph` | [Selection] | Repeat | Zero | none | terminal: Move or extend the review selection within captured output |
| `find-next-char` | [Operand, Selection] | Reject | One | none | terminal: Find the supplied character within captured output |
| `find-previous-char` | [Operand, Selection] | Reject | One | none | terminal: Find the supplied character within captured output |
| `find-till-next-char` | [Operand, Selection] | Reject | One | none | terminal: Find the supplied character within captured output |
| `find-till-previous-char` | [Operand, Selection] | Reject | One | none | terminal: Find the supplied character within captured output |
| `page-up` | [Selection, View] | Repeat | Zero | none | terminal: Move the review cursor by a page of captured output; media: Move to the previous or next PDF page |
| `page-down` | [Selection, View] | Repeat | Zero | none | terminal: Move the review cursor by a page of captured output; media: Move to the previous or next PDF page |
| `half-page-up` | [Selection, View] | Repeat | Zero | none | terminal: Move the review cursor by half a page of captured output; media: Move to the previous or next PDF page; directory-tree: Move the selected tree row by half a page |
| `half-page-down` | [Selection, View] | Repeat | Zero | none | terminal: Move the review cursor by half a page of captured output; media: Move to the previous or next PDF page; directory-tree: Move the selected tree row by half a page |
| `enter-insert-mode` | [Mode, Selection] | Reject | Zero | none | terminal: Return to live output and send subsequent input to the child |
| `enter-replace-mode` | [Mode, Selection] | Reject | Zero | none |  |
| `append-after` | [Mode, Selection] | Reject | Zero | none | terminal: Return to live output and send subsequent input to the child |
| `insert-line-start` | [Mode, Selection] | Reject | Zero | none | terminal: Return to live output and send subsequent input to the child |
| `insert-line-end` | [Mode, Selection] | Reject | Zero | none | terminal: Return to live output and send subsequent input to the child |
| `open-line-below` | [Text, Mode, Selection] | Reject | Zero | none | terminal: Return to live output and send subsequent input to the child |
| `open-line-above` | [Text, Mode, Selection] | Reject | Zero | none | terminal: Return to live output and send subsequent input to the child |
| `replace-char` | [Operand, Text, Mode] | Reject | One | none |  |
| `surround-add` | [Operand, Text, Mode] | Reject | One | none |  |
| `surround-replace` | [Operand, Text, Mode] | Reject | Two | none |  |
| `surround-delete` | [Operand, Text, Mode] | Reject | One | none |  |
| `toggle-case` | [Text, Mode] | Reject | Zero | none |  |
| `selection-undo` | [Selection] | Repeat | Zero | none | terminal: Restore the previous or next terminal review selection |
| `selection-redo` | [Selection] | Repeat | Zero | none | terminal: Restore the previous or next terminal review selection |
| `undo` | [Text, TerminalInput] | Repeat | Zero | none | terminal: Ask the child to erase the last text Runyte sent, if it has not been submitted |
| `redo` | [Text] | Repeat | Zero | none |  |
| `yank` | [Register] | Reject | Zero | none | terminal: Copy the review selection to the selected register and retain its selection and mode; directory: Copy selected entries and record a directory copy payload |
| `yank-line` | [Register] | Reject | Zero | none | directory: Copy selected entry lines and record a directory copy payload |
| `yank-collapse-normal-mode` | [Register, Selection, Mode] | Reject | Zero | none | terminal: Copy the review selection, retain it on screen and enter Normal mode; directory: Copy selected entries, collapse the selection and enter Normal mode |
| `yank-line-collapse-normal-mode` | [Register, Selection, Mode] | Reject | Zero | none | directory: Copy selected entry lines, collapse the selection and enter Normal mode |
| `paste-after-or-replace` | [Text, Mode, TerminalInput] | Repeat | Zero | none | terminal: Send register text to the child and enter Insert mode after successful queueing; directory: Stage copies or moves from a directory register into the listing |
| `paste-after` | [Text, TerminalInput] | Repeat | Zero | none | terminal: Send register text to the child without changing mode; directory: Stage copies or moves from a directory register into the listing |
| `replace-with-register` | [Text, Mode] | Reject | Zero | none | directory: Insert rows from a directory-register payload; directory pastes use row insertion rather than text replacement |
| `paste-before` | [Text, Mode, TerminalInput] | Repeat | Zero | none | terminal: Send register text to the child and enter Insert mode after successful queueing; directory: Stage copies or moves from a directory register into the listing |
| `indent` | [Text] | Reject | Zero | none |  |
| `unindent` | [Text] | Reject | Zero | none |  |
| `toggle-comments` | [Text] | Reject | Zero | none |  |
| `yank-delete-selection` | [Register, Text, Mode] | Reject | Zero | none | directory: Record selected entries as a move payload without removing files; delete or change the listing selection |
| `yank-change-selection` | [Register, Text, Mode] | Reject | Zero | none | directory: Record selected entries as a move payload without removing files; delete or change the listing selection |
| `delete-selection` | [Text, Mode] | Reject | Zero | none |  |
| `change-selection` | [Text, Mode] | Reject | Zero | none |  |
| `enter-select-mode` | [Mode, Selection] | Reject | Zero | none | terminal: Toggle Select mode over captured terminal output; media: Toggle selection of PDF text or an image region |
| `select-line` | [Selection, Mode] | Repeat | Zero | none | terminal: Select or extend captured output lines, entering Select mode for the line selection |
| `extend-line-above` | [Selection, Mode] | Repeat | Zero | none | terminal: Select or extend captured output lines, entering Select mode for the line selection |
| `extend-line-below` | [Selection, Mode] | Repeat | Zero | none | terminal: Select or extend captured output lines, entering Select mode for the line selection |
| `select-line-up` | [Selection, Mode] | Repeat | Zero | none | terminal: Select or extend captured output lines, entering Select mode for the line selection |
| `select-all` | [Selection, Mode] | Reject | Zero | none | terminal: Select all captured terminal output and enter Select mode |
| `collapse-selection` | [Selection, Mode] | Reject | Zero | none | terminal: Collapse the review selection and enter Normal mode |
| `flip-selection` | [Selection, Mode] | Reject | Zero | none |  |
| `expand-syntax-selection` | [Selection, Mode] | Repeat | Zero | none |  |
| `shrink-syntax-selection` | [Selection, Mode] | Repeat | Zero | none |  |
| `select-syntax-parent` | [Selection, Mode] | Repeat | Zero | none |  |
| `select-syntax-child` | [Selection, Mode] | Repeat | Zero | none |  |
| `select-previous-syntax-sibling` | [Selection, Mode] | Repeat | Zero | none |  |
| `select-next-syntax-sibling` | [Selection, Mode] | Repeat | Zero | none |  |
| `select-syntax-function` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-syntax-function` | [Selection, Mode] | Reject | Zero | none |  |
| `select-syntax-class` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-syntax-class` | [Selection, Mode] | Reject | Zero | none |  |
| `select-syntax-parameter` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-syntax-parameter` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-parentheses` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-parentheses` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-square-brackets` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-square-brackets` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-braces` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-braces` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-angle-brackets` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-angle-brackets` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-double-quotes` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-double-quotes` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-single-quotes` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-single-quotes` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-backticks` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-backticks` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-word` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-word` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-long-word` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-long-word` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-paragraph` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-paragraph` | [Selection, Mode] | Reject | Zero | none |  |
| `select-around-closest-delimiter` | [Selection, Mode] | Reject | Zero | none |  |
| `select-inside-closest-delimiter` | [Selection, Mode] | Reject | Zero | none |  |
| `goto-previous-syntax-function` | [Selection] | Repeat | Zero | none |  |
| `goto-next-syntax-function` | [Selection] | Repeat | Zero | none |  |
| `goto-previous-syntax-class` | [Selection] | Repeat | Zero | none |  |
| `goto-next-syntax-class` | [Selection] | Repeat | Zero | none |  |
| `goto-previous-syntax-parameter` | [Selection] | Repeat | Zero | none |  |
| `goto-next-syntax-parameter` | [Selection] | Repeat | Zero | none |  |
| `document-outline` | [View] | Reject | Zero | none |  |
| `toggle-syntax-fold` | [View] | Reject | Zero | none |  |
| `fold-all-syntax` | [View] | Reject | Zero | none |  |
| `unfold-all-syntax` | [View] | Reject | Zero | none |  |
| `split-selection-at-line-ends` | [Selection, Mode] | Reject | Zero | none |  |
| `split-selection-at-line-starts` | [Selection, Mode] | Reject | Zero | none |  |
| `keep-primary-selection` | [Selection] | Reject | Zero | none | terminal: Keep only the primary review selection |
| `remove-primary-selection` | [Selection] | Reject | Zero | none |  |
| `copy-selection-down` | [Selection] | Reject | Zero | none | terminal: Copy the review cursor to the adjacent captured output line |
| `copy-selection-up` | [Selection] | Reject | Zero | none | terminal: Copy the review cursor to the adjacent captured output line |
| `copy-selection-down-padded` | [Text, Selection] | Reject | Zero | none |  |
| `copy-selection-up-padded` | [Text, Selection] | Reject | Zero | none |  |
| `rotate-selection-forward` | [Selection] | Reject | Zero | none | terminal: Select the next or previous terminal review search match |
| `rotate-selection-backward` | [Selection] | Reject | Zero | none | terminal: Select the next or previous terminal review search match |
| `rotate-selection-contents-forward` | [Text, Selection] | Reject | Zero | none |  |
| `rotate-selection-contents-backward` | [Text, Selection] | Reject | Zero | none |  |
| `keep-matching-selections` | [Prompt, Selection] | Reject | Zero | none |  |
| `remove-matching-selections` | [Prompt, Selection] | Reject | Zero | none |  |
| `align-selections` | [Text, Selection] | Reject | Zero | none |  |
| `trim-selections` | [Selection] | Reject | Zero | none |  |
| `trim-trailing-whitespace` | [Text, Selection] | Reject | Zero | none |  |
| `hard-wrap` | [Text] | Reject | Zero | none |  |
| `reflow` | [Text] | Reject | Zero | none |  |
| `join-selections` | [Prompt, Text] | Reject | Zero | none |  |
| `join-lines` | [Text] | Reject | Zero | none |  |
| `format-table` | [Text] | Reject | Zero | none |  |
| `search` | [Prompt, SearchState, Selection, Mode] | Reject | Zero | none | terminal: Prompt for a case-insensitive search of captured output |
| `search-regex` | [Prompt, SearchState, Selection, Mode] | Reject | Zero | none | terminal: Prompt for a regular-expression search of captured output; directory-tree: Prompt to search visible tree entries |
| `search-next` | [Selection, Mode] | Reject | Zero | none | terminal: Select the next or previous review match without changing mode; directory-tree: Select the next or previous matching tree entry |
| `search-previous` | [Selection, Mode] | Reject | Zero | none | terminal: Select the next or previous review match without changing mode; directory-tree: Select the next or previous matching tree entry |
| `search-selection` | [SearchState, Selection, Mode] | Reject | Zero | none |  |
| `set-search-from-selection` | [SearchState] | Reject | Zero | none |  |
| `media-show-page` | [View, Mode, Selection] | Reject | Zero | none | pdf-pages: Display the selected PDF page and enter Normal mode |
| `media-zoom-in` | [View] | Reject | Zero | none |  |
| `media-zoom-out` | [View] | Reject | Zero | none |  |
| `media-fit` | [View] | Reject | Zero | none |  |
| `media-actual-size` | [View] | Reject | Zero | none |  |
| `media-pan-left` | [View] | Reject | Zero | none |  |
| `media-pan-right` | [View] | Reject | Zero | none |  |
| `media-move-up` | [View, Selection] | Repeat | Zero | none |  |
| `media-move-down` | [View, Selection] | Repeat | Zero | none |  |
| `media-copy-selection` | [Register] | Reject | Zero | none |  |
| `media-back` | [View, Mode, Selection] | Reject | Zero | none | media: Enter Normal mode, then fit media, clear selection or leave preview; pdf-pages: Collapse a Select-mode page selection, otherwise open the parent explorer |
| `media-select-all` | [Selection] | Reject | Zero | none |  |
| `align-view-center` | [View] | Reject | Zero | none | media: Center the image or PDF in its pane |
| `align-view-top` | [View] | Reject | Zero | none |  |
| `align-view-bottom` | [View] | Reject | Zero | none |  |
| `align-view-middle` | [View] | Reject | Zero | none | media: Center the image or PDF in its pane |
| `scroll-view-down` | [View] | Repeat | Zero | none | terminal: Capture review and scroll its output; media: Pan the image or PDF vertically |
| `scroll-view-up` | [View] | Repeat | Zero | none | terminal: Capture review and scroll its output; media: Pan the image or PDF vertically |
| `goto-window-top` | [Selection] | Reject | Zero | none | terminal: Move the review cursor within the visible captured output; directory-tree: Select the tree row at the top, middle or bottom of the view |
| `goto-window-center` | [Selection] | Reject | Zero | none | terminal: Move the review cursor within the visible captured output; directory-tree: Select the tree row at the top, middle or bottom of the view |
| `goto-window-bottom` | [Selection] | Reject | Zero | none | terminal: Move the review cursor within the visible captured output; directory-tree: Select the tree row at the top, middle or bottom of the view |
| `goto-file` | [File, View, Prompt, Background] | Reject | Zero | none | terminal: Open a path or URL inferred from the review selection and terminal directory |
| `goto-word` | [Operand, Selection] | Reject | Zero | none | terminal: Label visible captured output words and jump to one; directory-tree: Label visible tree entries and select one |
| `toggle-soft-wrap` | [View, Configuration] | Reject | Zero | none |  |
| `toggle-whitespace` | [View, Configuration] | Reject | Zero | none |  |
| `preview-document` | [Background, View] | Reject | Zero | none |  |
| `toggle-markdown-render` | [View] | Reject | Zero | none |  |
| `toggle-focus` | [View, Configuration] | Reject | Zero | none |  |
| `toggle-zen` | [View] | Reject | Zero | none |  |
| `toggle-fullscreen` | [View] | Reject | Zero | none |  |
| `open-explorer` | [File, View, Prompt] | Reject | Zero | none |  |
| `toggle-directory-tree` | [Pane, View, Mode] | Reject | Zero | none |  |
| `focus-directory-tree` | [Pane, View, Mode] | Reject | Zero | none |  |
| `directory-tree-up` | [Selection] | Reject | Zero | none |  |
| `directory-tree-down` | [Selection] | Reject | Zero | none |  |
| `directory-tree-left` | [View, Selection] | Reject | Zero | none |  |
| `directory-tree-right` | [View, Selection] | Reject | Zero | none |  |
| `directory-tree-first` | [Selection] | Reject | Zero | none |  |
| `directory-tree-last` | [Selection] | Reject | Zero | none |  |
| `directory-tree-page-up` | [Selection] | Reject | Zero | none |  |
| `directory-tree-page-down` | [Selection] | Reject | Zero | none |  |
| `directory-tree-open` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-close` | [Pane, View, Mode] | Reject | Zero | none |  |
| `directory-tree-refresh` | [File, View] | Reject | Zero | none |  |
| `directory-tree-legend` | [View] | Reject | Zero | none |  |
| `directory-tree-open-vertical` | [File, View, Mode, Prompt, Background, Pane] | Reject | Zero | none |  |
| `directory-tree-open-horizontal` | [File, View, Mode, Prompt, Background, Pane] | Reject | Zero | none |  |
| `directory-tree-pane-1` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-2` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-3` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-4` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-5` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-6` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-7` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-8` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-pane-9` | [File, View, Mode, Prompt, Background] | Reject | Zero | none |  |
| `directory-tree-new` | [Prompt, File] | Reject | Zero | none |  |
| `directory-tree-rename` | [Prompt, File] | Reject | Zero | none |  |
| `directory-tree-delete` | [Prompt, File] | Reject | Zero | none |  |
| `directory-tree-move` | [Prompt, File] | Reject | Zero | none |  |
| `open-working-directory-explorer` | [File, View, Prompt] | Reject | Zero | explorer [path] |  |
| `open-file-picker` | [View, Background] | Reject | Zero | none |  |
| `open-all-files-picker` | [View, Background] | Reject | Zero | none |  |
| `open-path-file-picker` | [Prompt, View, Background] | Reject | Zero | file-picker-path [path] |  |
| `open-directory-file-picker` | [View, Background] | Reject | Zero | none |  |
| `open-fuzzy-grep` | [View, Background] | Reject | Zero | none |  |
| `open-directory-fuzzy-grep` | [View, Background] | Reject | Zero | none |  |
| `open-directory-entry` | [File, View, Mode, Selection, Prompt, Background] | Reject | Zero | none |  |
| `open-parent-directory` | [File, View, Mode, Selection, Prompt] | Reject | Zero | none |  |
| `refresh-directory` | [File, View, Prompt] | Reject | Zero | none |  |
| `toggle-hidden-files` | [View, Configuration] | Reject | Zero | none | directory-tree: Show or hide hidden tree entries |
| `toggle-directory-details` | [View, Configuration] | Reject | Zero | none |  |
| `choose-explorer-order` | [Prompt, View, Configuration] | Reject | Zero | none |  |
| `open-explorer-system` | [File, View, Background] | Reject | Zero | none |  |
| `open-explorer-finder` | [File, View, Background] | Reject | Zero | none |  |
| `open-changed-file` | [Background, File, View, Prompt] | Reject | Zero | none |  |
| `stage-all-changed-files` | [Background, File, View] | Reject | Zero | none |  |
| `checkout-branch` | [Background, File, View, Prompt] | Reject | Zero | none |  |
| `create-branch` | [Prompt, Background, File] | Reject | Zero | none |  |
| `delete-branch` | [Prompt, Background, File] | Reject | Zero | none |  |
| `git-merge-branch` | [Background, File, View, Prompt, Mode] | Reject | Zero | none |  |
| `git-conflicts` | [Background, File, View, Mode] | Reject | Zero | none |  |
| `open-git-conflict` | [Background, File, View] | Reject | Zero | none |  |
| `git-merge-continue` | [Background, File, View, Prompt, Mode] | Reject | Zero | none |  |
| `git-merge-abort` | [Prompt, Background, File, Mode] | Reject | Zero | none |  |
| `git-conflict-next` | [Selection, View] | Reject | Zero | none |  |
| `git-conflict-previous` | [Selection, View] | Reject | Zero | none |  |
| `git-conflict-keep-current` | [Text] | Reject | Zero | none |  |
| `git-conflict-take-other` | [Text] | Reject | Zero | none |  |
| `git-conflict-sides` | [Background, File, View] | Reject | Zero | none |  |
| `git-conflict-resolve-literal-markers` | [Prompt, Background, File, Mode] | Reject | Zero | none |  |
| `git-conflict-resolve` | [Prompt, Background, File, Mode] | Reject | Zero | none |  |
| `git-conflict-list` | [Background, File, View, Mode] | Reject | Zero | none |  |
| `git-conflict-file-current` | [Prompt, Background, File, Mode] | Reject | Zero | none |  |
| `git-conflict-file-other` | [Prompt, Background, File, Mode] | Reject | Zero | none |  |
| `git-review-approve` | [View, Background, File, Mode] | Reject | Zero | none |  |
| `git-review-cancel` | [View] | Reject | Zero | none |  |
| `git-review-back` | [View] | Reject | Zero | none |  |
| `git-review-next` | [Selection] | Reject | Zero | none |  |
| `git-review-previous` | [Selection] | Reject | Zero | none |  |
| `git-review-left` | [Selection] | Reject | Zero | none |  |
| `git-review-right` | [Selection] | Reject | Zero | none |  |
| `git-review-enter` | [View, Background, File, Mode] | Reject | Zero | none |  |
| `git-fetch-branch` | [Background, File, View] | Reject | Zero | none |  |
| `pull-branch` | [Background, File, View, Prompt] | Reject | Zero | none |  |
| `push-branch` | [Background, File, View] | Reject | Zero | none |  |
| `open-worktree` | [Background, File, View] | Reject | Zero | none |  |
| `create-worktree` | [Prompt, Background, File] | Reject | Zero | none |  |
| `create-new-worktree` | [Prompt, Background, File] | Reject | Zero | none |  |
| `remove-worktree` | [Prompt, Background, File] | Reject | Zero | none |  |
| `next-git-network-page` | [Background, File, View, Mode] | Reject | Zero | none |  |
| `previous-git-network-page` | [View, Mode] | Reject | Zero | none |  |
| `next-git-log-page` | [Background, File, View, Mode] | Reject | Zero | none |  |
| `previous-git-log-page` | [Background, File, View, Mode] | Reject | Zero | none |  |
| `open-git-commit` | [Background, File, View, Mode] | Reject | Zero | none |  |
| `open-workspace-search-result` | [View, File, Selection] | Reject | Zero | none |  |
| `override-setting-language` | [Prompt, Configuration] | Reject | Zero | none |  |
| `override-setting-pattern` | [Prompt, Configuration] | Reject | Zero | none |  |
| `remove-setting-override` | [Configuration, View] | Reject | Zero | none |  |
| `activate-setting` | [Prompt, Configuration] | Reject | Zero | none |  |
| `open-settings` | [View, Mode] | Reject | Zero | none |  |
| `open-theme-settings` | [Configuration, View] | Reject | Zero | theme [name] |  |
| `split-vertical` | [Pane, Mode, File, Prompt, Background] | Reject | Zero | vsplit [path] |  |
| `split-horizontal` | [Pane, Mode, File, Prompt, Background] | Reject | Zero | hsplit [path] |  |
| `save` | [File, Text, Prompt, Background] | Reject | Zero | write [path] | directory: Review and apply edits to the directory listing as filesystem operations; commit-message: Review staged changes and submit the commit message |
| `force-save` | [File, Text, Prompt, Background] | Reject | Zero | write! [path] | directory: Review and apply edits to the directory listing as filesystem operations; commit-message: Review staged changes and submit the commit message |
| `show-help` | [View, Mode] | Reject | Zero | help [topic] | directory-tree: Open directory-tree help |
| `show-about` | [View] | Reject | Zero | none |  |
| `show-tutorial` | [View, Pane, Mode] | Reject | Zero | tutorial [reset&#124;sessions&#124;next] |  |
| `focus-window-left` | [Pane, Mode] | Reject | Zero | none |  |
| `focus-window-down` | [Pane, Mode] | Reject | Zero | none |  |
| `focus-window-up` | [Pane, Mode] | Reject | Zero | none |  |
| `focus-window-right` | [Pane, Mode] | Reject | Zero | none |  |
| `next-window` | [Pane, Mode] | Reject | Zero | none |  |
| `swap-window` | [Pane] | Reject | Zero | none |  |
| `move-window-left` | [Pane] | Reject | Zero | none |  |
| `move-window-down` | [Pane] | Reject | Zero | none |  |
| `move-window-up` | [Pane] | Reject | Zero | none |  |
| `move-window-right` | [Pane] | Reject | Zero | none |  |
| `close-window` | [Pane, Mode] | Reject | Zero | none | directory-tree: Close the directory tree and restore pane focus |
| `only-window` | [Pane] | Reject | Zero | none |  |
| `equalize-windows` | [Pane] | Reject | Zero | none |  |
| `open-terminal` | [Background, View, Mode] | Reject | Zero | terminal [command] |  |
| `open-terminal-file-directory` | [Background, View, Mode] | Reject | Zero | terminal-file-directory [command] |  |
| `open-terminal-directory-root` | [Background, View, Mode] | Reject | Zero | terminal-directory-root [command] |  |
| `open-terminal-selected-directory` | [Background, View, Mode] | Reject | Zero | terminal-selected-directory [command] |  |
| `open-terminal-session-directory` | [Background, View, Mode] | Reject | Zero | terminal-session-directory <number&#124;name> |  |
| `open-terminal-list` | [View] | Reject | Zero | none |  |
| `show-terminal` | [View, Mode] | Reject | Zero | terminal-show <number&#124;name> |  |
| `rename-terminal` | [Prompt, Configuration] | Reject | Zero | terminal-rename <name> |  |
| `leave-terminal` | [View, Mode] | Reject | Zero | none |  |
| `copy-terminal-output` | [View] | Reject | Zero | none |  |
| `send-to-terminal` | [TerminalInput] | Reject | Zero | terminal-send [number&#124;name] |  |
| `delete-word-backward` | [Text] | Reject | Zero | none |  |
| `delete-word-forward` | [Text] | Reject | Zero | none |  |
| `delete-to-line-start` | [Text] | Reject | Zero | none |  |
| `delete-to-line-end` | [Text] | Reject | Zero | none |  |
| `delete-char-backward` | [Text] | Reject | Zero | none |  |
| `delete-char-forward` | [Text] | Reject | Zero | none |  |
| `insert-newline` | [Text] | Reject | Zero | none |  |
| `insert-indent` | [Text] | Reject | Zero | none |  |
| `insert-other-indent` | [Text] | Reject | Zero | none |  |
| `commit-undo-checkpoint` | [History] | Reject | Zero | none |  |
| `goto-definition` | [Background, View] | Reject | Zero | none |  |
| `goto-declaration` | [Background, View] | Reject | Zero | none |  |
| `goto-type-definition` | [Background, View] | Reject | Zero | none |  |
| `goto-references` | [Background, View] | Reject | Zero | none |  |
| `goto-implementation` | [Background, View] | Reject | Zero | none |  |
| `new-buffer` | [View] | Reject | Zero | none |  |
| `open-navigator` | [View] | Reject | Zero | none |  |
| `previous-destination` | [View] | Reject | Zero | none |  |
| `previous-session` | [File, View, Background] | Reject | Zero | none |  |
| `session-1` | [File, View, Background] | Reject | Zero | none |  |
| `session-2` | [File, View, Background] | Reject | Zero | none |  |
| `session-3` | [File, View, Background] | Reject | Zero | none |  |
| `session-4` | [File, View, Background] | Reject | Zero | none |  |
| `session-5` | [File, View, Background] | Reject | Zero | none |  |
| `session-6` | [File, View, Background] | Reject | Zero | none |  |
| `session-7` | [File, View, Background] | Reject | Zero | none |  |
| `session-8` | [File, View, Background] | Reject | Zero | none |  |
| `session-9` | [File, View, Background] | Reject | Zero | none |  |
| `next-running-session` | [File, View, Background] | Reject | Zero | none |  |
| `previous-running-session` | [File, View, Background] | Reject | Zero | none |  |
| `open-session-directory` | [File, View, Background] | Reject | Zero | none |  |
| `open-explorer-session` | [File, View, Background] | Reject | Zero | none |  |
| `open-explorer-workspace` | [File, View, Background] | Reject | Zero | none |  |
| `open-buffer-picker` | [View] | Reject | Zero | none |  |
| `list-accept` | [View, Prompt] | Reject | Zero | none |  |
| `list-open-actions` | [View, Prompt] | Reject | Zero | none |  |
| `list-backspace` | [View] | Reject | Zero | none |  |
| `list-next` | [Selection] | Reject | Zero | none |  |
| `list-previous` | [Selection] | Reject | Zero | none |  |
| `list-page-down` | [Selection] | Reject | Zero | none |  |
| `list-page-up` | [Selection] | Reject | Zero | none |  |
| `list-first` | [Selection] | Reject | Zero | none |  |
| `list-last` | [Selection] | Reject | Zero | none |  |
| `list-toggle-preview` | [View] | Reject | Zero | none |  |
| `list-close-target` | [View, Prompt, Background, Text, File] | Reject | Zero | none |  |
| `list-clear-filter` | [View] | Reject | Zero | none |  |
| `list-close` | [View] | Reject | Zero | none |  |
| `open-session-destinations` | [File, View, Background] | Reject | Zero | none |  |
| `global-search` | [Prompt, Background, View] | Reject | Zero | none |  |
| `global-search-regex` | [Prompt, Background, View] | Reject | Zero | none |  |
| `show-documentation` | [Background, View] | Reject | Zero | none |  |
| `document-symbols` | [Background, View] | Reject | Zero | none |  |
| `workspace-symbols` | [Prompt, Background, View] | Reject | Zero | none |  |
| `diagnostics` | [View] | Reject | Zero | none |  |
| `trigger-completion` | [Background, Prompt, Text, Mode] | Reject | Zero | none |  |
| `jump-backward` | [View, Selection] | Reject | Zero | none |  |
| `jump-forward` | [View, Selection] | Reject | Zero | none |  |
| `jump-backward-buffer` | [View, Selection] | Reject | Zero | none |  |
| `jump-forward-buffer` | [View, Selection] | Reject | Zero | none |  |
| `rename-symbol` | [Background, Prompt, Text] | Reject | Zero | none |  |
| `code-action` | [Background, Prompt, Text] | Reject | Zero | none |  |
| `clipboard-paste-after` | [Text, Mode, TerminalInput] | Repeat | Zero | none | terminal: Send system clipboard text to the child without changing mode |
| `clipboard-paste-before` | [Text, Mode, TerminalInput] | Repeat | Zero | none | terminal: Send system clipboard text to the child without changing mode |
| `clipboard-yank` | [Register] | Reject | Zero | none | terminal: Copy the captured review selection to the system clipboard; media: Copy selected PDF text or an image region |
| `clipboard-paste` | [Text, TerminalInput] | Reject | Zero | none | terminal: Paste clipboard text or an image reference into the terminal input queue |
| `select-register` | [Operand, Register] | Reject | One | none |  |
| `record-macro` | [Operand, Macro] | Reject | One | none |  |
| `record-default-macro` | [Macro] | Reject | Zero | none |  |
| `stop-macro-recording` | [Macro] | Reject | Zero | none |  |
| `replay-macro` | [Operand, Macro, Text] | Repeat | One | none |  |
| `replay-default-macro` | [Macro, Text] | Repeat | Zero | none |  |
| `list-macros` | [View] | Reject | Zero | none |  |
| `shell-pipe` | [Prompt, Background, Text] | Reject | Zero | none |  |
| `match-bracket` | [Selection] | Reject | Zero | none |  |
| `cd` | [File, Configuration, Prompt, Background, View, Application] | Reject | Zero | cd [path] |  |
| `init` | [File, Configuration, Background, View, Application] | Reject | Zero | init [directory] |  |
| `close` | [View] | Reject | Zero | none |  |
| `diff-disk` | [File, View] | Reject | Zero | none |  |
| `diff-remote` | [Background, View] | Reject | Zero | none |  |
| `diff-off` | [View] | Reject | Zero | none |  |
| `diff-this` | [View] | Reject | Zero | none |  |
| `close!` | [View] | Reject | Zero | none |  |
| `format` | [Background, Text] | Reject | Zero | none |  |
| `git-compare` | [Background, View] | Reject | Zero | none |  |
| `git-branches` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-blame` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-blame-file` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-cancel` | [Background] | Reject | Zero | none |  |
| `git-commit` | [Prompt, Background, File, View, Mode] | Reject | Zero | none |  |
| `git-diff` | [Background, View] | Reject | Zero | none |  |
| `git-diff-side-by-side` | [Background, View] | Reject | Zero | none |  |
| `git-discard` | [Prompt, Background, File] | Reject | Zero | none |  |
| `git-index` | [Background, View] | Reject | Zero | none |  |
| `git-log` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-network` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-network-head` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-network-all` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-network-ref` | [Background, View, Mode] | Reject | Zero | git-network-ref <full-ref> |  |
| `git-network-choose-ref` | [View] | Reject | Zero | none |  |
| `git-search-commits` | [Background, View] | Reject | Zero | none |  |
| `git-refresh` | [Background, View] | Reject | Zero | none |  |
| `git-stage` | [Background, File] | Reject | Zero | none |  |
| `git-status` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-stashes` | [Background, View, Mode] | Reject | Zero | none |  |
| `git-stash-tracked` | [Prompt, Background, File] | Reject | Zero | git-stash-tracked <name> |  |
| `git-stash-all` | [Prompt, Background, File] | Reject | Zero | git-stash-all <name> |  |
| `git-stash-untracked` | [Prompt, Background, File] | Reject | Zero | git-stash-untracked <name> |  |
| `git-stash-apply` | [Prompt, Background, File] | Reject | Zero | none |  |
| `git-stash-drop` | [Prompt, Background, File] | Reject | Zero | none |  |
| `git-stage-hunk` | [Background, File] | Reject | Zero | none |  |
| `git-unstage-hunk` | [Background, File] | Reject | Zero | none |  |
| `git-stage-lines` | [Background, File] | Reject | Zero | none |  |
| `git-unstage` | [Background, File] | Reject | Zero | none |  |
| `git-worktrees` | [Background, View, Mode] | Reject | Zero | none |  |
| `config-reload` | [Configuration, Background] | Reject | Zero | none |  |
| `write-config` | [File] | Reject | Zero | write-config [path] |  |
| `write-config!` | [File] | Reject | Zero | write-config! [path] |  |
| `grammar` | [Configuration, Mode, Selection] | Reject | Zero | grammar [runyte] |  |
| `lsp-trust` | [Prompt, Configuration, Background] | Reject | Zero | none |  |
| `mcp` | [Prompt, Configuration, Background] | Reject | Zero | mcp [identity] |  |
| `agent-channel` | [Background, View] | Reject | Zero | none |  |
| `lsp-restart` | [Background] | Reject | Zero | lsp-restart [language] |  |
| `log-open` | [View] | Reject | Zero | none |  |
| `lsp-status` | [Background, View] | Reject | Zero | none |  |
| `notifications` | [View, Mode] | Reject | Zero | none |  |
| `open` | [File, View, Prompt, Background] | Reject | Zero | open <path> |  |
| `path` | [View] | Reject | Zero | none |  |
| `describe-key` | [Prompt, View, Mode] | Reject | Zero | none |  |
| `action-browser` | [View, Mode] | Reject | Zero | none |  |
| `copy-key-binding` | [Register] | Reject | Zero | none |  |
| `detach` | [Application] | Reject | Zero | none |  |
| `quit` | [Pane, View, Application] | Reject | Zero | none |  |
| `quit!` | [Pane, View, Application] | Reject | Zero | none |  |
| `quit-all` | [Application] | Reject | Zero | none |  |
| `quit-all!` | [Application] | Reject | Zero | none |  |
| `quit-all!!` | [Application] | Reject | Zero | none |  |
| `quit-here` | [Application] | Reject | Zero | none |  |
| `quit-here!` | [Application] | Reject | Zero | none |  |
| `reload` | [Prompt, Background, File, Text] | Reject | Zero | none |  |
| `resize` | [Pane, Prompt, Mode] | Reject | Zero | none |  |
| `service-health` | [View] | Reject | Zero | none |  |
| `pipe` | [Background, Text] | Reject | Zero | pipe <shell-command> |  |
| `pipe-cancel` | [Background] | Reject | Zero | none |  |
| `plugins` | [View] | Reject | Zero | none |  |
| `plugin-stop` | [Background] | Reject | Zero | plugin-stop <configured-id> |  |
| `plugin-restart` | [Background] | Reject | Zero | plugin-restart <configured-id> |  |
| `write-quit` | [File, Text, View, Application, Prompt, Background] | Reject | Zero | none |  |
| `write-buffer-close` | [File, Text, View, Prompt, Background] | Reject | Zero | none |  |
| `session-attach` | [Background, View] | Reject | Zero | session-attach <workspace> |  |
| `session-list` | [Background, View] | Reject | Zero | none |  |
| `session-stop` | [Background] | Reject | Zero | session-stop [workspace] |  |
| `session-rename` | [Background, Configuration] | Reject | Zero | session-rename <workspace> <name> |  |
| `session-clean` | [Background] | Reject | Zero | none |  |
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

Native document previews use part of your `scopes.media.normal` bindings:
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
