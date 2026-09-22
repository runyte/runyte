# `Ctrl-\` does not switch an integrated terminal to Normal or review on Windows

On Windows, pressing `Ctrl-\` in an integrated terminal does not switch from
terminal Insert to live Normal mode or reach Normal/review mode. The child
program and any effect it receives from the key have not been recorded.

To reproduce, open an integrated terminal with `:terminal` on Windows, focus
its pane in Insert mode, and press `Ctrl-\`. Press it a second time to test the
review transition. The expected behavior is for the first press to enter live
Normal mode without freezing child output, and for the second press to capture
the terminal's review snapshot. `i` should return to terminal input.

The Windows version, outer terminal, shell, keyboard layout, and whether
Runyte receives `Ctrl-\`, its legacy `Ctrl-4` spelling, or another input event
have not been established. Check the native console input report and editor
key dispatch before attributing the failure to either layer.
