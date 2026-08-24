# macOS terminal pane keys require an intermediate mode change

In a terminal pane on macOS, directional pane navigation does not complete in
the first key sequence while the pane is in Insert mode.

With `editor.fast_pane_keys` enabled, each of `Ctrl-h`, `Ctrl-j`, `Ctrl-k`, and
`Ctrl-l` changes the editor from Insert to Normal on the first press. A second
press is required to move to the adjacent pane. On Linux the first press moves
immediately.

The prefixed form also differs from Linux. Pressing `Ctrl-w` first changes
Insert to Normal; pressing `h`, `j`, `k`, or `l` after it then enters terminal
review instead of moving to the adjacent pane.

Both forms are expected to execute the directional focus command immediately
from Terminal Insert. A terminal destination should start live Insert, while a
document destination should start Normal. Neither form should capture terminal
review or send its control bytes to the child.

Commit `31b13b9` requested the keyboard protocol's disambiguation-only profile
on macOS while retaining legacy repeat detection. Testing on macOS confirmed
that this did not change the reported behavior, so protocol availability alone
is not the cause. The native Crossterm key identities for the affected chords
still need to be captured at Runyte's frontend boundary.

The correction must retain the macOS safeguard against unreliable enhanced
keyboard repeat and release events.
