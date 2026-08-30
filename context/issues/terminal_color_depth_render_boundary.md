# The rendering API can bypass outer-terminal colour adaptation

The public `ui::render` entry point renders with
`TerminalColorDepth::TrueColor` unconditionally. Production frontends are
expected to call `render_with_color_depth`, while presentation-oriented test
backends use `render` to retain exact RGB values.

That distinction is expressed only in documentation and naming. A new
production call site can compile while calling `render`, silently bypassing
the outer terminal's detected colour depth. On a basic or 256-colour terminal
this can emit colours outside the advertised capability and make a persistent
session render differently depending on which entry point a caller happened
to choose.

The production rendering boundary should require an explicit
`TerminalColorDepth`. The exact-RGB helper should either be restricted to test
code or have a name that makes its test/presentation-neutral purpose explicit.
The same audit should cover `render_host_frame`, which also has a TrueColor
default beside its colour-depth-aware form.

The host must continue to retain exact RGB in semantic snapshots; adaptation
belongs to each attached client and must not mutate editor or persistent
session state. Existing test backends may keep an exact-colour route, but a
future production caller must not be able to select it accidentally. Key
dispatch, snapshot ownership, and terminal capability detection are outside
the scope of this change.

Regression coverage should render representative RGB and ANSI colours at
TrueColor, 256-colour, and basic depths through the public frontend boundary.
The basic-depth cases must preserve the documented `White` to `Gray` and
`DarkGray` to `Black` mapping.
