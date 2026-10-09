# Ctrl+left-click does not open links across native views

Native Runyte supports opening file paths and web URLs with `gf`, but terminal
content requires switching from input mode to terminal review mode first
(described as preview mode in the report). Links should also be interactive
through the mouse without requiring that mode change.

Ctrl+left mouse click should perform the equivalent of `gf` on the target
under the mouse pointer in every native view that displays a supported file
path or URL. This includes terminal panes in both review and input mode.
File paths should use the existing `gf` navigation behavior, and web URLs
should open in the default browser. Target detection and resolution should
follow the existing `gf` rules, including supported wrapped terminal links.

The clicked location, rather than an unrelated caret or selection, should
determine the target. In terminal input mode, opening a link must not require
entering review mode, and the handled gesture must not also be forwarded to
the terminal child as mouse input. Existing `gf` keyboard navigation should
remain available.

To reproduce, display a file path or web URL in a native editor view and in
the output of an integrated terminal session. Verify that `gf` can open the
target, using terminal review mode where necessary. Return the terminal to
input mode and Ctrl+left-click the target. The expected behavior is the same
navigation or browser action directly from the clicked link, without first
switching modes. Repeat in terminal review and other native views displaying
supported targets.
