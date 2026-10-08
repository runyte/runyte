# Native window system clipboard copy and paste do not integrate

System clipboard copy and paste are unavailable in the native window. Even
copying text with `Space c y` does not allow it to be pasted back.
`Ctrl+Shift+c` and `Ctrl+Shift+v` should provide system clipboard copy and paste
as in Alacritty, including in integrated terminals. Ordinary Ctrl+c must
remain available to terminal programs. Existing editor clipboard commands
must continue to work.
