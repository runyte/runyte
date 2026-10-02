# Terminal sessions draw ANSI colours from the outer terminal's palette

Editor panes and terminal panes take their colours from two unrelated
palettes.

- Buffers, the gutter, diagnostics, Git marks, selections and overlays use the
  active theme's RGB roles.
- In a terminal session, a cell coloured with one of the sixteen ANSI colours
  (SGR 30–37, 40–47, 90–97, 100–107, and `38;5;n`/`48;5;n` with `n` < 16) is
  stored as `Color::Indexed(n)`. On 256-colour and true-colour outer terminals,
  `terminal_color` in `src/ui.rs` emits it unchanged as an indexed colour, so
  the outer terminal emulator's own profile decides the actual colour. Only
  the default foreground and background (`Color::Default`) come from the theme
  (`terminal_style` in `src/ui.rs`).

The result is a mismatch on one screen:

- An error diagnostic in a buffer uses the theme's `error`, while the same
  compiler error printed in a terminal pane next to it uses the outer
  terminal's ANSI red. Warnings, Git added/removed lines (`change_added`,
  `change_removed` against `git diff` output) and similar meanings differ the
  same way.
- The terminal pane's ground follows the theme, but its ANSI foregrounds were
  tuned by the outer terminal profile for some other ground. With a light
  theme in a terminal profile designed for dark backgrounds, ANSI yellow,
  white, bright cyan and bright green can become close to invisible on the
  pane background. A dark theme in a light-tuned profile has the reverse
  problem with black and dark blue.

Reproduction:

1. Run Runyte in an outer terminal whose profile is designed for a dark
   background, and switch to a light theme such as `ocean-light` or `latte`.
2. Open a terminal session and run:

   ```sh
   for i in 0 1 2 3 4 5 6 7; do
     printf '\033[3%sm normal %s \033[9%sm bright %s\033[0m\n' "$i" "$i" "$i" "$i"
   done
   ```

3. Compare those colours with the theme's `error`, `warning`, `info` and
   `change_*` colours in a buffer with diagnostics and Git changes.

Expected: by default, the sixteen ANSI colours in terminal sessions come from
the active theme, so a terminal pane matches the editor panes around it and
stays readable on the theme's ground. A boolean setting turns this off and
restores the current pass-through to the outer terminal's palette.

## Setting

- A boolean in `Space o o`, `true` by default. The name is not decided yet. It
  belongs with the existing groups in the settings reference in
  `docs/user-guide.md`, for example `editor.terminal_theme_colors`.
- It applies without a restart. Toggling it, or switching the theme while it
  is on, redraws existing terminal sessions in the new colours, since the
  stored cells keep their index and only the presentation changes.
- `false` keeps today's behaviour exactly: the indices go to the outer
  terminal as they are.

## Theme palette

- Every theme gets sixteen ANSI colours: black, red, green, yellow, blue,
  magenta, cyan and white, plus their bright forms.
- A custom theme can set them under a new optional key, for example
  `terminal: { black: …, red: …, …, bright_white: … }`, with the final shape to
  be decided. An unknown name is a configuration error, as unknown syntax
  scopes are.
- Omitted slots are derived from the theme's own roles, so a custom theme
  that sets none still matches its editor panes. A plausible derivation: red
  from `error`, yellow from `warning`, green from `change_added` or `info`,
  blue and cyan from `accent`/`directory`, magenta from `change_modified`,
  black and white from `background` and `foreground`, with bright forms
  stepped from those. The exact derivation is not decided.
- Built-in themes whose upstream projects publish terminal palettes
  (Catppuccin, Everforest, Nightfox, Zenbones, GitHub, Atom One, gruvbox,
  base16) use those palettes. Runyte's own themes (`ocean-*`, `ember-*`,
  `dark`, `light`, `matrix`, `neon`, `paper`) need palettes chosen to match
  their existing roles. Each needs to stay legible on its own ground,
  including the light themes.
- The theme's semantic roles and the ANSI slots carrying the same meaning
  should agree, so an error, warning or added line has one hue whether it
  comes from the editor or from a program's output.

## Constraints

- Only indices 0–15 are remapped. Indices 16–255 and 24-bit RGB are explicit
  choices made by the program and stay as they are.
- Colour depth stays client-owned, as described under "Outer terminal colour
  depth" in `context/reference/terminal-compatibility-v1.md`. A persistent
  session host keeps the index in its snapshots and protocol frames; each
  attached client resolves it through the theme and then adapts the RGB value
  to its own depth. On an outer terminal that only advertises basic colours,
  theme-resolved colours fall back to the nearest basic colour, the same as
  other RGB theme roles.
- A theme whose `background` is `reset` leaves the ground to the outer
  terminal. Whether such a theme should still remap the sixteen colours, or
  pass them through because the ground they must contrast with is unknown, is
  not decided.
- `OSC 4` palette queries and setters are ignored today, and the
  terminal-compatibility reference records that. Whether a query should be
  answered from the theme palette while the setting is on is not decided.
  Setters stay ignored either way.
- Update the Themes section and the settings reference in
  `docs/user-guide.md`, the colour-query notes in the integrated terminal
  section, and `context/reference/terminal-compatibility-v1.md`.
