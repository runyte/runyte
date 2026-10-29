## Configuration

Runyte reads one YAML file at startup. Every field is optional: an empty or
missing file gives the defaults described below.

There are three ways to change a setting:

- **In the editor.** `Space o o` opens the `[config]` page, where you pick a
  value and Runyte saves it to the file for you.
- **In the file.** Edit the YAML in any editor, then run `:config-reload`.
- **On the command line.** `runyte --config <path>` loads a different file.

Key bindings and themes have their own sections:
[Custom key bindings](custom-key-bindings.md#custom-key-bindings) and [Themes](themes.md#themes).

### Where the configuration lives

Runyte uses the first of these that applies:

| Condition | Path |
| --- | --- |
| `XDG_CONFIG_HOME` is set (any platform) | `$XDG_CONFIG_HOME/runyte/config.yaml` |
| Otherwise, on Linux and macOS | `~/.config/runyte/config.yaml` |
| Otherwise, on Windows | `%APPDATA%\runyte\config.yaml` |

- Empty environment values are ignored. If no platform default can be
  resolved, Runyte starts without a configuration file.
- `--config <path>` replaces the default path. A relative path is resolved
  from the directory Runyte was launched in, even if workspace setup later
  moves to another directory.
- [config.example.yaml](../../config.example.yaml) is a complete, commented
  starting point.

### What the file looks like

The file is a set of top-level sections. A small configuration:

```yaml
editor:
  tab_width: 2
  soft_wrap: true

workspace:
  mode: persistent

theme: mocha
```

| Section | What it configures | Reference |
| --- | --- | --- |
| `editor` | Editing, display, explorer, and completion behavior | [Editor settings](configuration.md#editor-settings) |
| `workspace` | Workspace state and persistent sessions | [Workspace settings](configuration.md#workspace-settings) |
| `git` | Automatic Git refresh | [Other settings](configuration.md#other-settings) |
| `notifications` | Notification history | [Other settings](configuration.md#other-settings) |
| `lsp` | Language servers | [Language servers](language-support.md#language-servers) |
| `keys` | Prefixes, remapped keys, and action bindings | [Custom key bindings](custom-key-bindings.md#custom-key-bindings) |
| `theme` | The active theme | [Themes](themes.md#themes) |
| `themes` | Custom theme definitions | [Writing a custom theme](themes.md#writing-a-custom-theme) |
| `plugins` | Process plugins | [Plugin guide](../plugins.md) |

### Themes and key bindings in their own files

Two directories beside the configuration file add to it:

| Directory | Each `.yaml` file holds |
| --- | --- |
| `themes.d/` | One theme, named after the file: `themes.d/mine.yaml` defines the theme `mine`. The file is what would go under `themes: mine:`. |
| `keybinds.d/` | Part of the `keys` section, written as it would be under `keys:`. |

```yaml
# keybinds.d/normal.yaml
bind:
  normal:
    F5: save
```

- They are read after the configuration file, in file-name order. A later
  file wins: a theme replaces one of the same name, including a built-in or
  one defined inline under `themes:`. Key files merge one binding at a time,
  so a later file can change one key in `bind` or `rebind` without repeating
  the rest; `leader` and `window` are replaced.
- Only files ending in `.yaml` are read. Hidden files, such as an editor's
  swap files, are skipped, and so is a directory that does not exist. At most
  256 files are read from each.
- They are found beside whichever file was loaded, including one given with
  `--config`, and are read even when that file does not exist.
- Inline `themes:` and `keys:` sections keep working alongside them.
- A theme file that is not valid YAML stops Runyte from starting, as the
  configuration file does. A key file in the wrong shape is reported like
  any other `keys` problem and never stops it.

### Changing settings in the editor

Open the settings page with `Space o o`, `:config`, or `:settings`.

1. Move to a setting. The page is a read-only buffer, so motions, selections,
   search, splits, and your key bindings all work.
2. Press Enter anywhere on its line, including a wrapped continuation, to open
   a value popup.
3. Pick a value. For settings that apply immediately, moving through a list
   previews each choice live. Restart-required settings are not previewed.
4. Press Enter to save, or Escape to cancel and roll the preview back.

What the popups offer:

- **Lists** for booleans, themes, and settings with a fixed set of values.
- **Typed input** for numbers. The popup shows the allowed minimum and maximum
  and rejects anything outside them.

What saving does to your file:

- The file is patched atomically, and only the changed value changes.
  Comments, key order, and unknown fields stay as they were.
- Other YAML flow collections (`{...}` and `[...]`), even multiline ones, are
  kept.
- The setting being changed must be a plain value in a block mapping. Editing
  inside a flow mapping is not supported.
- YAML that cannot be patched without loss is refused, and the file is left
  untouched. A failed save also rolls back the preview and keeps the popup
  open so you can retry.

How the page looks:

- Three left-aligned columns: setting, saved value, and description. Setting
  names use the theme's function colour, values its constant colour, and
  descriptions ordinary text.
- The setting and value columns are as wide as their content, two spaces
  apart. Descriptions have no inserted line breaks or trailing padding.
- Each setting is one line. It follows `editor.soft_wrap`: `Space p s` toggles
  wrapping, and horizontal scrolling reaches long lines when wrapping is off.
  Wrapped lines use ordinary buffer layout; they do not align under the
  description column.

Most changes apply at once. A few only take effect after a restart; the page
says so when you save them:

| Setting | Why it needs a restart |
| --- | --- |
| `editor.mouse` | Mouse capture is set up when the terminal is opened. |
| `editor.font_size` | The native window reads its font size when it opens. |
| `lsp.enable` | Language servers are started or suppressed at startup. |
| `mode` | The launch mode is chosen before the editor starts. The saved value applies to future launches. |

The `keys` section is never written by this page. Edit it in the file.

### Indentation overrides

In `Space o o`, move to `editor.tab_width` or `editor.indent` and press Tab.
Choose **Override for language…**, select a language, then enter the width or
choose spaces/tabs. Or choose **Override for file pattern…**, enter a pattern,
then set the value. Widths remain limited to 1–16.

Only saved overrides appear beneath the setting. Enter edits an override; Tab
opens **Remove override**. Escape cancels, including any style preview. An
explicit override equal to the global default stays explicit, so later changes
to the default do not change it. Removal writes `null` for that field, preserving
comments and rule order; the row disappears and the field inherits again.

The value popup identifies the inherited value and source. A file pattern can
cover several languages, so it has no single inherited value. When the document
from which settings were opened matches the pattern, the popup shows that
file's inherited value; otherwise it explains the varying inheritance and shows
the global default.

The same rules can be authored in YAML:

```yaml
editor:
  tab_width: 4
  indent: spaces
indentation:
  languages:
    python:
      tab_width: 4
    yaml:
      tab_width: 2
  files:
    '**/generated/*.yaml':
      tab_width: 4
    'Makefile':
      indent: tabs
```

Each field inherits independently: global default, language override, then
matching file patterns in their YAML order. The last matching rule that sets a
field wins. Editing an existing rule keeps its position; a new rule is appended.
There are no built-in language overrides or content-based indentation detection.

Patterns match paths relative to the workspace root, which does not change with
`:cd`. A pattern without `/` matches a filename at any depth. `*` matches within
a directory, `?` matches one character, and `**` spans directories; `**/` also
matches zero directories. Use `/` on every platform. Absolute paths, `..`,
backslashes, character classes and brace expansion are not supported. A pattern
may contain at most 256 bytes, and each scope allows at most 128 entries.
Matching does not scan the filesystem. Local files outside the workspace do not
match file rules. Pathless and provider documents can use their detected language
but have no local path for file rules.

Overrides apply to existing open buffers immediately. Removing one changes how
indentation is displayed and subsequently edited; it does not rewrite existing
text. Editing the YAML manually takes effect through `:config-reload`.

### Writing configuration

`:write-config [path]` writes the whole effective configuration: every setting
at its current value, the theme in use, configured plugins, and every
language server, including the built-in ones. Themes and keys go into
[files of their own](configuration.md#themes-and-key-bindings-in-their-own-files) beside it:

| File | What it holds |
| --- | --- |
| `themes.d/<name>.yaml` | Each theme that is not exactly a built-in one. Built-in themes are not written. |
| `keybinds.d/00-prefixes.yaml` | `leader`, `window`, `rebind`, and `defaults`, when any are set. |
| `keybinds.d/normal.yaml`, `select.yaml`, `insert.yaml`, `replace.yaml` | Explicit bindings under `bind`, with built-in defaults as commented examples. With `keys.defaults: false`, only the explicit bindings. |

A theme whose name is not a portable file name, or differs from another only
in letter case, stays inline under `themes:`. The built-in keys left out of
the mode files are the ones `bind` cannot express, such as count keys, and
`Ctrl-h`/`j`/`k`/`l`, so `editor.fast_pane_keys` keeps choosing those. Both
still come from the built-in keymap. Commented defaults stay supplied by Runyte,
so they can receive future default changes and do not consume the custom
binding allowance. Explicit assignments stay active, including assignments
that happen to match a current default, `null` removals, and action sequences.

For a mode with custom bindings, uncomment an example entry to customize it.
For a mode with no assignments, uncomment its `bind:` and mode headings too:

```yaml
# bind:
#   normal:
#     w: move-word-forward
```

An export with no custom keys leaves all four mode files as comments; loading
it does not activate custom-keymap compilation. Exporting is a way to inspect
and edit configuration, not a startup optimization.

Without a path it writes the per-user configuration file,
`$XDG_CONFIG_HOME/runyte/config.yaml` or `~/.config/runyte/config.yaml`
(`%APPDATA%\runyte\config.yaml` on Windows), even when this session loaded
another file with `--config`. A relative path is resolved from the working directory, `~` expands to
the home directory, and missing parent directories are created.

Nothing is written if any of these files already exists, and the refusal
names it; `:write-config! [path]` replaces them. Other files in `themes.d/`
and `keybinds.d/` are left alone and are still read. A symlink's target is
replaced and the link itself is kept, as a settings change does. Each file is
written atomically, with the configuration file last. Before writing, Runyte
validates the generated documents together with retained destination fragments,
in their load order. It checks both fast-pane-key variants and the 256-file
limit in each directory. A validation failure leaves all files untouched.
An I/O failure during writing can still leave a partial set of updated files;
the multi-file export is not one filesystem transaction.

What is written is the saved configuration. A setting read at startup and
changed since, such as `editor.mouse`, is written with its new value even
though the running editor keeps its launch value until a restart. The written
files contain generated keybinding examples, but do not preserve comments from
the original files. Maps are written in name order for deterministic output.

### Editing the file and reloading

After editing the file by hand, run `:config-reload`. It re-reads the file
this editor loaded, including one given with `--config`.

- Nothing is watched or polled. The reload happens when you run the command.
- It covers only the workspace you run it in. Run it again in another
  running session to update that one.

**Failed reloads change nothing.** Invalid YAML, an out-of-range value, and a
deleted file are reported, and the running configuration stays exactly as it
was.

- A file replaced atomically (written elsewhere, then renamed) is always read
  whole.
- A file rewritten in place can be caught half-written. That fails as invalid
  YAML; reload again once the write finishes.
- A path that never had a file is not treated as deleted. The reload reads
  `themes.d` and `keybinds.d` beside it, so editing a key file there takes
  effect without a configuration file. Create the file and reload to start
  using it mid-session.

**What applies immediately:** everything except the restart settings below.
That includes the theme, `notifications.history_limit`, and the `keys`
section. Key dispatch, help, and key hints are rebuilt together, with each
running plugin's keys laid back over them.

**What keeps its startup value:** `editor.mouse`, `editor.font_size`, `lsp.enable`,
`mcp.default_grants`, `mode`, `workspace.state`, and `workspace.state_anchor`. The status
line names them as needing a restart. The `[config]` page shows the file's
value as saved, while the editor keeps using the startup value.

**Partial failures:**

- A `keys` section that collides with a running plugin's binding is refused
  on its own. The rest of the file still applies.
- A `theme` that cannot be built keeps the current theme on screen and says
  so.

**Language servers are reconfigured, not restarted.**

- A server whose `command`, `args`, `initialization_options`, and `roots` are
  unchanged keeps its process, handshake, open documents, and diagnostics.
- A changed definition stops and starts again on the next request.
- A language removed from the file stops.
- A remembered launch failure is forgotten when its definition changes, so a
  corrected command is tried again.

**Plugins are matched by `id`**, so moving an entry within the file changes
nothing.

- An unchanged entry keeps its running process.
- A changed entry restarts its plugin. A removed or disabled entry stops it.
- An added entry starts. One that was disabled or refused at startup can be
  fixed and started the same way.
- A plugin that is busy keeps running until its work is done. Busy means a
  running job, an activity lease, a helper process, outstanding cleanup, an
  unsaved remote document, an unanswered command, local filesystem work, or a
  prompt you are part-way through.
- While a plugin waits, `:plugins` shows the pending state and a notification
  says so. The saved entry applies by itself when the work is gone.
  `:plugin-restart <id>` applies it sooner, or `:plugin-stop <id>` for an
  entry the file no longer enables.

### Settings reference

Every setting below is also on the `[config]` page, except `workspace.state`,
`workspace.state_anchor`, `mcp.default_grants`, and the language-server
definitions, which are file-only.

#### Editor settings

| Setting | Default | Values | What it does |
| --- | --- | --- | --- |
| `editor.grammar` | `runyte` | `runyte` (`helix` is an alias) | Editing grammar. The alias is for compatibility, not a promise of full Helix behavior. |
| `editor.line_numbers` | `true` | boolean | Show line numbers beside editable buffers. |
| `editor.relative_line_numbers` | `false` | boolean | Show document-line distances from each pane's cursor, keeping the cursor line absolute. Requires `editor.line_numbers`. Wrapped continuations keep their arrows; folded lines still count in distances. |
| `editor.tab_width` | `4` | 1–16 | Display and indentation width of a tab. |
| `editor.indent` | `spaces` | `spaces`, `tabs` | Style of new indentation. `Tab` inserts this style, `Shift-Tab` the other. |
| `editor.smart_newline` | `true` | boolean | Add syntax indentation and continue Markdown lists on a new line. `false` keeps only the leading indent. |
| `editor.auto_close` | `false` | boolean | Pair typed brackets and quotes in Insert mode. |
| `editor.scroll_offset` | `3` | 0–100 | Rows kept visible above and below the cursor. |
| `editor.motion_repeat_multiplier` | `2` | 1–10 | Motions per held-key repeat. `1` keeps terminal speed. See [Held-key speed](configuration.md#held-key-speed). |
| `editor.directory_tree_width` | `33` | 12–240 | Preferred directory tree width in columns. |
| `editor.show_hidden_files` | `false` | boolean | Show dotfiles in the explorer, finder, and workspace search. `.` toggles it in an explorer. |
| `editor.explorer_sort` | `name` | `name`, `modified`, `size`, each also `_descending` | Explorer order. Directories always come first. |
| `editor.explorer_details` | `false` | boolean | Show `ls -l` style mode, owner, size, and time columns. `?` toggles it in an explorer. |
| `editor.soft_wrap` | `false` | boolean | Wrap long lines visually without changing the text. |
| `editor.always_wrap_rendered_markdown` | `true` | boolean | Always wrap rendered Markdown pages, including rendered scratchpads. When false, follow `editor.soft_wrap`. Does not affect Markdown source. |
| `editor.render_whitespace` | `false` | boolean | Show `·` for spaces, `→` for tabs, and `¬` for line endings. |
| `editor.zen_width` | `100` | 1–1000 | Maximum text width while `:zen` is active. |
| `editor.hard_wrap_width` | `80` | 1–1000 | Width used by `Space p w` (hard wrap) and `Space p r` (reflow). |
| `editor.scratch_markdown` | `true` | boolean | Treat the pathless scratch buffer as Markdown for `Space p r` and `?`. |
| `editor.trim_trailing_whitespace` | `true` | boolean | Remove spaces and tabs at line ends when saving. |
| `editor.font_size` | `15` | `8`–`48` | Native window font size in logical pixels. Read when a window opens; Ctrl-plus/minus change only the current window. |
| `editor.mouse` | `true` | boolean | Capture the mouse for selection, scrolling, and pane resizing. `false` keeps the terminal's own text selection. Restart required. |
| `editor.word_completion` | `true` | boolean | Suggest words already open elsewhere in the workspace. |
| `editor.word_completion_minimum` | `3` | 1–32 | Prefix length before word suggestions appear. |
| `editor.fast_pane_keys` | `false` | boolean | `Ctrl-h/j/k/l` move between panes without the `Ctrl-w` prefix. |
| `editor.selecting_motions` | `true` | boolean | `w/b/e/W/B/E` and `f/t/F/T` select what they cross, as in Helix. `false` moves a caret instead. |
| `editor.auto_close_terminal` | `true` | boolean | Forget terminal sessions and their output when the program exits. `false` retains future exited sessions for review in the Finder and `Space t t`. |
| `editor.terminal_theme_colors` | `true` | boolean | Use the active theme for terminal ANSI colours 0–15. Applies immediately, including existing output. |
| `editor.preview_theme_colors` | `true` | boolean | Paint `:preview` Markdown, data and source pages in the active theme's colours. `false` keeps the light page. Applies immediately to open previews. |
| `editor.command_mode_dim` | `true` | boolean | Gray out pane text while a command prompt is open. |

#### Workspace settings

| Setting | Default | Values | What it does |
| --- | --- | --- | --- |
| `mode` | `ide` | `editor`, `ide`, `mux` | The mode of a launch with no `--editor`, `--ide`, or `--mux`. `editor` needs no workspace; `mux` makes a bare `runyte` start or reuse the project's host. The deprecated `workspace.mode` (`standalone`, `persistent`) is still read, with a notification naming its replacement. Restart required. |
| `workspace.session_strip` | `auto` | `auto`, `always`, `hidden` | When to show the row of running persistent sessions. `auto` shows it when more than one is running. |
| `workspace.idle_retirement_minutes` | `0` | 0–43200 | Minutes a clean, unattached host stays alive. `0` keeps hosts running. Applies without restarting the host. |
| `workspace.state` | `.runyte` | path | Where the workspace keeps runtime state. `workspace.root` is an accepted alias. See [Where workspace state lives](configuration.md#where-workspace-state-lives). |
| `workspace.state_anchor` | unset | `profile`, `local-app-data` | Windows only. See [Windows state anchor](configuration.md#windows-state-anchor). |

`:session-stop` stops a host explicitly, whatever the retirement interval.

#### Other settings

| Setting | Default | Values | What it does |
| --- | --- | --- | --- |
| `git.refresh_interval_seconds` | `60` | 0–3600 | Bounds automatic Git refresh. `0` turns it off. See [Automatic Git refresh](configuration.md#automatic-git-refresh). |
| `notifications.history_limit` | `50` | 1–1000 | Newest notifications kept in memory for the workspace. |
| `lsp.enable` | `true` | boolean | Allow configured language servers to start. Restart required. |
| `mcp.default_grants` | none | identity: list of permissions | Identities that may read in every workspace that has no grant or revocation of its own. The only permission is `read`; `write` is granted per workspace with `:mcp`. Earlier capability names are accepted; a list with both `terminal_read` and `editor_context_read` means `read`, and any other grants nothing and is reported at start. Read when a workspace starts. See [Default grants](agent-access-to-workspace-context.md#default-grants). |
| `lsp.<language>` | `rust` only | `command`, `args`, `initialization_options`, `roots` | A language server definition. See [Language servers](language-support.md#language-servers). |
| `theme` | `ocean-dark` | theme name | The active theme. See [Themes](themes.md#themes). |

### Configuration recipes

Each recipe is a complete file. Combine the parts you need.

**Indent with tabs, shown eight columns wide:**

```yaml
editor:
  indent: tabs
  tab_width: 8
```

**Write prose comfortably:**

```yaml
editor:
  soft_wrap: true
  hard_wrap_width: 72 # Space p w and Space p r wrap here
  zen_width: 80 # :zen centres text at this width
  auto_close: false
```

**Show hidden files and list the newest first, with details:**

```yaml
editor:
  show_hidden_files: true
  explorer_sort: modified_descending
  explorer_details: true
```

**Keep the terminal's native mouse selection:**

```yaml
editor:
  mouse: false # restart required
```

**Move between panes with `Ctrl-h/j/k/l`:**

```yaml
editor:
  fast_pane_keys: true
```

This takes `Ctrl-j` and `Ctrl-k` from Insert mode. It also takes all four keys
from terminal programs, where `Ctrl-h` is backspace and `Ctrl-l` clears the
screen.

**Plain carets instead of Helix-style selecting motions:**

```yaml
editor:
  selecting_motions: false
```

**Use persistent sessions and retire idle hosts after two hours:**

```yaml
workspace:
  mode: persistent # restart required
  session_strip: always
  idle_retirement_minutes: 120
```

**Refresh Git less often, or never automatically:**

```yaml
git:
  refresh_interval_seconds: 300 # or 0 to rely on :git-refresh
```

**Add a language server:**

```yaml
lsp:
  markdown:
    command: marksman
    args: ["server"]
```

**Turn language servers off entirely:**

```yaml
lsp:
  enable: false # restart required
```

### Setting details

#### Held-key speed

`editor.motion_repeat_multiplier` applies only to single-key movements that
the terminal repeats while the key is held, such as the arrow keys and
`h`, `j`, `k`, and `l`. A single press still moves once.

- Runyte asks terminals that support it for enhanced key reporting, so a held
  key can be told apart from repeated presses.
- On macOS it asks only for unambiguous key codes and detects repeats from
  their timing. Some terminals there have reported ordinary keys as repeats.
- For terminals that report repeats as ordinary presses, Runyte recognises
  the long initial delay followed by the steady repeat cadence, and applies
  the same multiplier.

#### Automatic Git refresh

Git views and tracked-file gutters refresh from filesystem events while they
are visible. `git.refresh_interval_seconds` bounds this work in both
directions:

- **At most this often.** Event-driven refreshes never start more often than
  the interval.
- **At least this often.** After the interval, a periodic check catches
  anything a lost or unsupported event missed.

Details:

- An event that arrives while no Git view is visible waits until one is
  shown.
- Showing a different Git view waits for the interval before fetching more
  automatically.
- `:git-refresh` and Runyte's own Git operations are never rate-limited.
- `0` turns off both event-driven and periodic refresh. `:git-refresh`, and
  the refresh that follows Runyte's own Git operations, still work.

#### Where workspace state lives

`workspace.state` may be absolute, or relative to the workspace directory.

Runyte finds the workspace directory by walking up from the launch directory:

1. It looks for the nearest Git, Jujutsu, or Subversion working-copy root.
2. If there is none, it looks for the configured relative state directory
   (`.runyte/` by default).
3. If neither exists, an ide launch creates the state directory in the launch
   directory and uses that as the workspace. It refuses instead in the
   directories listed under [Finding the
   workspace](workspaces-and-persistent-sessions.md#starting-a-workspace), for a
   target outside the launch directory, and whenever `workspace.state` is
   absolute, since discovery cannot find an absolute state directory again.
   mux refuses too. Both suggest `runyte --init DIRECTORY`. [Editor
   mode](workspaces-and-persistent-sessions.md#editor-mode) needs no workspace.

`runyte --init DIRECTORY` creates the state directory, so later launches find
the same workspace.

Because discovery walks upward, initializing your home directory makes it the
workspace for every directory below it that has no Git repository and no state
directory of its own.

#### Windows state anchor

On Windows, `workspace.state_anchor` sets the boundary for durable plugin
state:

```yaml
workspace:
  state_anchor: profile # or local-app-data
```

- The boundary comes from the current account's known-folder record.
- `workspace.state` must be strictly inside that boundary.
- Existing parent directories are used as they are. Only the final state
  directory is created, as private storage.
- Custom anchors, relocating existing state, and falling back to another
  ancestor are not supported.
- Leaving it unset keeps the full-ancestry behavior.
- Unix accepts the setting, so one file can be shared, but ignores it.

### Checking service health

`Space o s` or `:service-health` opens a read-only report on optional
services. It shows:

- syntax registry failures,
- the active document's language server configuration and attachment,
- Git repository discovery, including a remembered failure or pending retry,
- the diagnostic log's owner, level, path, and any failure.

It only probes paths, so it still works when every optional service is
missing. In mux mode the `log` row describes the host that owns the
workspace, not the client that opened the report.

The report is an informational overlay: arrows and paging scroll it, and
Escape dismisses it. Printable keys and Enter cannot turn it into a filtered
choice by accident.

<!-- navigation -->
[Contents](README.md) · [Previous: Diagnostics and logging](diagnostics-and-logging.md) · [Next: Custom key bindings](custom-key-bindings.md)
