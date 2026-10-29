## Themes

Runyte starts in `ocean-dark`. Pick another built-in theme, or define your own
in the configuration file.

### Choosing a theme

| How | What it does |
| --- | --- |
| `:theme mocha` | Switch to a theme by name. |
| `Space o t` or `:theme` | Open the theme list. Moving through it previews each theme. |
| `theme` row in `Space o o` | The same list, from the settings page. |
| `theme: mocha` in the file | Set the theme directly, then run `:config-reload`. |

- A theme chosen in the editor is written to `theme:` in the configuration
  file, so Runyte starts in it next time.
- In the theme list, `Tab` narrows to dark themes, then light themes, then
  back to all. The popup title names the group shown. Typing filters within
  the group.
- Dark and light are judged from each theme's background, so your own themes
  are grouped the same way.

```yaml
theme: everforest-dark-medium
```

### Built-in themes

| Family | Dark | Light |
| --- | --- | --- |
| Ocean (default) | `ocean-dark` | `ocean-light` |
| Ember | `ember-dark` | `ember-light` |
| Neutral | `dark` | `light` |
| Classic | `base16`, `gruvbox` | `paper` |
| Runyte originals | `matrix`, `neon` | |
| Atom and GitHub | | `atom-one-light`, `github-light` |
| Catppuccin | `frappe`, `macchiato`, `mocha` | `latte` |
| Everforest | `everforest-dark-hard`, `everforest-dark-medium`, `everforest-dark-soft` | `everforest-light-hard`, `everforest-light-medium`, `everforest-light-soft` |
| Nightfox | `nordfox`, `nordfox-warm`, `terafox`, `terafox-soft` | |
| Zenbones | `zenbones-dark`, `zenwritten-dark`, `neobones-dark`, `rosebones-dark`, `forestbones-dark`, `tokyobones-dark`, `seoulbones-dark`, `nordbones-dark`, `nordbones-dark-soft`, `duckbones-dark`, `zenburned-dark`, `kanagawabones-dark` | `zenbones-light`, `zenwritten-light`, `neobones-light`, `rosebones-light`, `forestbones-light`, `tokyobones-light`, `seoulbones-light`, `vimbones-light` |

#### About the themes

**`ocean-dark` and `ocean-light`** are one palette seen from two grounds, drawn
from a breaking wave: deep water is the ground, the wave's turquoise face is
the accent, and foam and sky are what the palette lightens toward.

- Every role is the same hue at the same contrast in both variants.
- Both are deliberately soft: text reads at 8.6:1 against the background
  rather than 13:1, the step `terafox-soft` takes. Keywords and strings sit
  just below ordinary text, and comments at 3.9:1.
- They cannot soften further without text becoming hard to read on the shared
  Git diff grounds.
- Mode carets and status labels use turquoise Normal, coral Insert, apricot
  Select, indigo Command, and orchid Replace. Insert and Select take their
  red/orange pairing from `terafox`, adjusted for the ocean grounds.
- Other warm colours are the error red, which one-key jump labels share,
  the warning amber, and the Git gutter and diff colours every built-in theme
  shares.

**`ember-dark` and `ember-light`** are Runyte's branded pair, named for the
red they carry on a neutral gray ground.

- Modes: green Normal, red Insert, pink Select, purple Replace, blue Command.
- They are the only built-in themes that set `command`, so palette command
  names are blue while the accent stays on borders and headings.
- Selections differ by hue alone: a blue secondary against a pink primary
  that matches the Select caret.
- The dark theme takes its surface, text, and accent from runyte.com. The
  light theme uses darker forms of the same hues on light gray.

**`dark` and `light`** are the neutral pair: no palette identity, just a
legible option for each kind of terminal.

**`matrix`** is a green-phosphor terminal theme.

- An almost-black ground with a green cast, a saturated code green for
  keywords and the accent, and a ladder of greens for everything else.
- Blue is rare on purpose, so blue things are the ones worth finding: azure
  for functions, directories, and Command mode; cyan for types; indigo for the
  Replace caret.
- Ordinary selections are deep green; the primary selection is blue.
- Only errors and one-key jump labels are red, apart from the Git gutter and
  diff colours every built-in theme shares.

**`neon`** is a near-black ground under a red frame and a cyan interior, shaped
like a heads-up game menu.

- Red draws structure: pane borders, keywords, tags, errors, and the resting
  caret.
- Cyan marks what you act on: calls, directories, and the Insert caret.
- Acid green is kept for strings.
- Text reads at 9.7:1, about as bright as `ocean-dark`; the ratio is higher
  only because the ground is darker.
- The red stays darker than the other colours. Lightening a saturated red to
  match them turns it pink and loses the frame.
- The primary selection is the Select caret's acid green, darkened to a
  ground and kept at the yellow end of green so it is not read as a Git
  "added" line. Ordinary selections are blue. A deep red selection was
  rejected because it looked like a deleted line.

**Nightfox themes** use the canonical palettes from
[Nightfox](https://github.com/EdenEast/nightfox.nvim).

- `nordfox` is a cool Nord-derived dark theme.
- `nordfox-warm` keeps that base with brighter dimmed text, pink secondary
  selections, and yellow primary selections.
- `terafox` uses deep blue-green backgrounds and warm accents.
- `terafox-soft` is Runyte's own variant: `terafox` with text lowered from
  13.1:1 to 8.6:1 contrast, for long reading without glare. Identifiers,
  operators, and punctuation move by the same amount; everything else is
  unchanged.

**`atom-one-light`** follows Atom's official
[One Light UI](https://github.com/atom/one-light-ui) and
[One Light syntax](https://github.com/atom/one-light-syntax) palettes.

**`github-light`** follows projekt0n's
[GitHub Theme for Neovim](https://github.com/projekt0n/github-nvim-theme).

**Zenbones themes** use the palettes and generated highlight colours from
[Zenbones](https://github.com/zenbones-theme/zenbones.nvim).

- Its dynamic `randombones` selector is intentionally not a Runyte theme.
- `nordbones-dark-soft` is Runyte's own variant: `nordbones-dark` with text
  lowered from 10.6:1 to 7:1 contrast. Its background, accents, and
  selections are unchanged.
- These themes keep their upstream Visual and Search selection backgrounds.

**Mode colours in most other built-in themes** are blue Normal, red Insert,
neon green Replace, orange Select, and purple Command. The Command and Replace
colours are Runyte's own rather than taken from an upstream palette.

**Selections in most of Runyte's original themes** pair a cool secondary
selection with a warm primary one.

**Directories** are usually dark blue in the built-in light themes and a
lighter palette blue in the dark ones.

### Writing a custom theme

Declare a theme under `themes`, then select it with `theme`:

```yaml
theme: midnight

themes:
  midnight:
    background: "#10131a"
    foreground: "#d8dee9"
    accent: "#88c0d0"
    syntax:
      comment: "#65737e"
      keyword: "#b48ead"
      string: "#a3be8c"
```

Every field is optional. An omitted field either takes a fixed default or
borrows another role's colour; the
[theme colour reference](themes.md#theme-colour-reference) says which.

A theme that sets most roles:

```yaml
theme: midnight

themes:
  midnight:
    background: "#10131a"
    foreground: "#d8dee9"
    muted: "#65737e"
    whitespace: "#292c33"
    accent: "#88c0d0"
    command: "#8be9fd"
    cursor_normal: "#ff5555"
    cursor_insert: "#ff79c6"
    cursor_replace: "#39ff14"
    cursor_select: "#ffb86c"
    cursor_secondary: "#d8dee9"
    cursor_command: "#bd93f9"
    directory: "#8be9fd"
    destination_file: "#d8dee9"
    destination_explorer: "#8be9fd"
    destination_generated: "#65737e"
    destination_scratch: "#5fd7e7"
    destination_terminal: "#8ddb8c"
    selection: "#2e3440"
    selection_primary: "#694b37"
    fuzzy_match_secondary: "#2e3440"
    fuzzy_match_primary: "#694b37"
    error: "#bf616a"
    warning: "#d08770"
    info: "#a3be8c"
    jump_label_immediate: "#ff5555"
    jump_label_primary: "#5fd7e7"
    jump_label_secondary: "#4ab7c6"
    change_added: "#8ddb8c"
    change_modified: "#d2a8ff"
    change_removed: "#ff7b72"
    diff_added: "#185435"
    diff_removed: "#642830"
    diff_changed: "#51316f"
    syntax:
      comment: "#65737e"
      keyword: "#b48ead"
      markup.heading: "#88c0d0"
      markup.italic: "#b48ead"
      markup.raw: "#a3be8c"
      string: "#a3be8c"
```

A custom theme is listed, previewed, and sorted into dark or light exactly
like a built-in one.

### Colour values

A colour is either:

- `#rrggbb`, or
- a terminal colour name: `black`, `red`, `green`, `yellow`, `blue`,
  `magenta`, `cyan`, `white`, `grey`, or `dark-grey`.

Runyte stores every colour as exact RGB and adapts it to the terminal when
drawing:

| Terminal advertises (`COLORTERM` or `TERM`) | Colours drawn |
| --- | --- |
| True colour | The exact palette |
| 256 colours | Nearest xterm cube or grayscale entry |
| Anything less | Nearest basic ANSI colour |

- The same adaptation applies to programs running in terminal panes.
- In 256 colours, pane and popup grounds that would land on the same entry are
  nudged apart, so the layers stay distinct.
- A persistent session host keeps the exact theme. Each attached client adapts
  it to its own terminal, so attaching from a lower-colour terminal does not
  change the workspace theme.

### Terminal ANSI colours

Terminal panes, frozen review and terminal previews use the active theme's
sixteen ANSI colours by default. Switching themes or toggling
`editor.terminal_theme_colors` in `Space o o` recolours existing output
immediately. Set it to `false` to use the outer terminal's palette instead.
Only indices 0–15 are remapped; indices 16–255 and explicit RGB retain the
program's colours, subject to the client's colour depth.

Custom themes can override any of these slots under `terminal`:
`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, and each
name prefixed with `bright_`. Unknown names and invalid colours are errors.
For example, inside a theme definition:

```yaml
terminal:
  red: "#e65c57"
  bright_red: "#f08a92"
```

Omitted normal slots use `muted`, `error`, `change_added`, `warning`,
`directory`, `change_modified`, `accent`, and `foreground`, respectively.
Omitted bright slots take those roles a further 12% toward white on dark
grounds or black on light grounds. Derived colours are moved toward the higher-contrast
black or white extreme to reach at least 3:1 on the theme ground; explicit overrides are used exactly.
An override of a normal slot does not change an omitted bright slot.

Bundled third-party themes start from their upstream terminal palettes;
Atom One Light uses its official syntax hues. Low-contrast entries are moved
toward white or black to reach 3:1 on the theme background, including neutral
slots upstream intended as backgrounds. Runyte's original themes derive their
palettes from the roles above. Upstream hues are retained, while Runyte's
shared Git roles remain unchanged; ANSI green and Git added marks need not
have identical RGB values.

A `reset` background still uses the theme palette. Its actual ground is
unknown, so contrast cannot be guaranteed; disable the setting to defer to
the outer terminal's palette. Named colour and `reset` overrides retain their
normal terminal-owned meaning. OSC 4 palette queries and setters are ignored.

### Theme colour reference

"Default" is what an omitted field becomes in a custom theme: a fixed colour,
or another role's colour.

- The fixed defaults are tuned for dark backgrounds. A light custom theme
  should set `selection`, `change_*`, `diff_*`, and `jump_label_*` itself, as
  well as the base colours.
- Setting `warning`, `info`, `change_*`, or `diff_*` to `null` uses a fallback
  instead of the fixed default: `warning` borrows `change_modified` and `info`
  borrows `change_added` (then terminal yellow and green); `change_*` use the
  terminal's green, magenta, and red; `diff_*` leave lines unfilled, so only
  the gutter marks show the comparison.

#### Base colours

| Role | Colours | Default |
| --- | --- | --- |
| `background` | Ground of the active pane | `#181818` |
| `foreground` | Ordinary text | `#d8d8d8` |
| `muted` | Dimmed interface text | `#585858` |
| `accent` | Pane and popup borders, headings | `#7cafc2` |
| `command` | Command names in the command palette | `accent` |
| `directory` | Directory entries in explorers (files use `foreground`) | `accent` |
| `whitespace` | The `·`, `→`, and `¬` markers | A dim step off `background`; `muted` if `background` is `reset` |

#### Mode carets

Each colours the primary caret and the status line's mode label in its mode.

| Role | Mode | Default |
| --- | --- | --- |
| `cursor_normal` | Normal | `accent` |
| `cursor_insert` | Insert | `error` |
| `cursor_replace` | Replace | Saturated neon green, darker on light grounds; neon magenta if another mode colour is green |
| `cursor_select` | Select | `warning` |
| `cursor_command` | Command | `info` |
| `cursor_secondary` | Secondary carets in a multi-selection | `foreground` |

- The primary caret keeps its mode colour in a multi-selection, or the pending
  replacement colour.
- If the terminal draws `cursor_secondary` the same as a primary caret,
  Runyte picks another available colour.

#### Selections and matches

| Role | Colours | Default |
| --- | --- | --- |
| `selection` | Secondary ranges in a multi-selection | `#383838` |
| `selection_primary` | The primary range and ordinary Select-mode ranges | `selection` |
| `fuzzy_match_secondary` | Individual characters of a scattered fuzzy match | `selection` |
| `fuzzy_match_primary` | A contiguous fuzzy match | `selection_primary` |

The fuzzy-match defaults make `Space s` matches use the same two colours as
selections.

#### Git and comparisons

| Role | Colours | Default |
| --- | --- | --- |
| `change_added` | Git gutter mark; added lines in a diff buffer; count columns in the changed-file list | `#8ddb8c` |
| `change_modified` | Git gutter mark | `#d2a8ff` |
| `change_removed` | Git gutter mark; removed lines in a diff buffer; count columns in the changed-file list | `#ff7b72` |
| `diff_added` | Line background, side-by-side comparison | `#185435` |
| `diff_removed` | Line background, side-by-side comparison | `#642830` |
| `diff_changed` | Line background, side-by-side comparison | `#51316f` |

- `change_*` are strong mark colours. `diff_*` are background tints, dark
  enough on dark themes and light enough on light themes to keep text
  readable.
- Every built-in theme uses high-contrast green, purple, and red for these, so
  a meaning keeps its hue everywhere.

#### Destination types

These colour the TYPE column of the Navigator, the buffer list, and the
terminal list.

| Role | Type | Default |
| --- | --- | --- |
| `destination_file` | `[file]` | `foreground` |
| `destination_explorer` | `[explorer]` | `directory` |
| `destination_generated` | Generated pages such as `[config]` or `[help]` | `muted` |
| `destination_scratch` | `[scratch]` | `jump_label_primary` |
| `destination_terminal` | `[terminal]` | `change_added` |

The STATE column reuses other roles:

| State | Role |
| --- | --- |
| `[+]` | `change_modified` |
| `[STALE]`, `bell` | `warning` |
| `[RO]`, `exited` | `muted` |
| `unread` | `info` |

#### Notification colours

| Role | Colours | Default |
| --- | --- | --- |
| `error` | Error headings and counts | `#ab4642` |
| `warning` | Warning headings and counts | `#dc9656` |
| `info` | Information headings and unread counts | `#a1b56c` |

#### Jump labels

These colour the `gw` (goto-word) labels.

| Role | Colours | Default |
| --- | --- | --- |
| `jump_label_immediate` | One-key labels for nearby targets. After the first key of a two-key label, the matching second keys move to their target cells in this colour. | `error` |
| `jump_label_primary` | First character of a two-key label | `#5fd7e7` |
| `jump_label_secondary` | Second character of a two-key label | `#4ab7c6` |
| `jump_text_muted` | Ordinary text in the active pane while labels are shown, and every pane while a command prompt is open | `muted` |

- Built-in themes use one neon-cyan hue for both two-key characters. The
  second is darker on dark backgrounds and lighter on light ones.
- The Zenbones light themes use a darker pair. The mid-gray `seoulbones-dark`
  and `zenburned-dark` use a brighter pair.
- `light` and `paper` use lighter jump-only grays for `jump_text_muted`.
- Dimming during command prompts covers `:` and the search, rename, and other
  text-entry prompts. Pending keys such as `g` and `Space` change nothing.
  Carets keep their mode colours, and the prompt and its completion list are
  never dimmed. Turn
  it off with `editor.command_mode_dim: false`.

The look of `gw` is inspired by
[`smoka7/hop.nvim`](https://github.com/smoka7/hop.nvim); Runyte's
implementation is independent.

#### Accepted but unused

`status_background` and `status_foreground` are accepted for compatibility.
The status line is drawn on `background` with `foreground` text.

### Syntax colours

`syntax` maps highlight scopes to colours:

```yaml
themes:
  midnight:
    syntax:
      keyword: "#b48ead"
      string: "#a3be8c"
      markup.heading: "#88c0d0"
```

**Code scopes:** `attribute`, `comment`, `constant`, `constructor`,
`function`, `keyword`, `label`, `namespace`, `number`, `operator`, `property`,
`punctuation`, `string`, `tag`, `type`, and `variable`.

**Markdown scopes:** `markup.heading`, `markup.bold`, `markup.italic`,
`markup.link.text`, `markup.link.url`, `markup.list`, `markup.quote`,
`markup.raw`, and `markup.strikethrough`.

**Diagnostic scopes:** `diagnostic.error` and `diagnostic.warning`.

- An omitted scope uses the theme's `foreground`, except `diagnostic.error`
  and `diagnostic.warning`, which use `error` and `warning`.
- An unknown scope name is a configuration error, not silently ignored.
- Tree-sitter captures use the longest scope that prefixes them, so
  `keyword.control.return` takes the `keyword` colour.
- Built-in themes colour every Markdown scope.

Some Markdown scopes also carry a text attribute, in rendered pages and in
source alike. The attribute belongs to the scope, not the theme; a theme only
picks the colour.

| Scope | Attribute |
| --- | --- |
| `markup.bold`, `markup.heading` | bold |
| `markup.italic` | italic |
| `markup.link.url` | underline |
| `markup.strikethrough` | crossed out |

### Derived colours

Some surfaces are computed from the theme rather than named in it, so every
theme, custom or built-in, gets them:

- **Inactive panes** use a ground halfway between `background` and the popup
  ground.
- **Popups** (pickers, choice lists, the command palette, action menus, key
  hints, and language-server popups) use a ground one small step off
  `background`: lighter on dark themes, darker on light ones. The step is
  small enough that both still read as the same colour.
- A theme whose `background` is `reset` leaves the ground to the terminal.
  That cannot be stepped off, so its popups keep the same background.

<!-- navigation -->
[Contents](README.md) · [Previous: Custom key bindings](custom-key-bindings.md) · [Next: Project layout](project-layout.md)
