# Frequently asked questions

## Why don't Alt-based keybindings work on macOS?

On macOS, Runyte's `Alt` bindings use the **Option (`⌥`)** key. For example,
`Alt-o` means holding Option while pressing `o`. Your terminal must send
Option as an Alt/Meta modifier; otherwise, macOS may use it to type special
characters or compose accents instead.

In Apple's **Terminal** app:

1. Open **Terminal → Settings → Profiles**.
2. Select the profile used by your terminal window.
3. Open **Keyboard** and enable **Use Option as Meta key**.
4. Return to Runyte and try **Option-o** or **Option-i** in Normal mode.

This setting applies to the selected Terminal profile and also affects other
programs running in windows that use it. Option combinations in those windows
will act as terminal shortcuts instead of typing their usual special
characters. See [Apple's Terminal keyboard settings guide](https://support.apple.com/guide/terminal/trmlkbrd/mac).

In Runyte's Normal and Select modes, `Alt-o` jumps backward to a different
buffer or terminal surface in navigation history, and `Alt-i` jumps forward.
They skip positions within the current buffer. A message such as
`no earlier buffer` or `no later buffer` means the shortcut was received, but
there is no destination in that direction.

See the [user guide](user-guide.md#key-bindings) for more about keybindings.
