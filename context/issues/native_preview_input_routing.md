# Rapid command input can dismiss a document preview

With a document preview open, a rapid `:hsplit plain.txt` can lose its first
letter and dismiss the preview. Linux Xvfb/lavapipe acceptance observed a dark
source pane after `Ctrl-Up`; the expected preview raster remained absent even
after ten seconds. The same failure occurred before helper integration.

A diagnostic key trace records `:` and `s` routed against the same painted
frame. The intervening `h` is consumed as preview navigation before command
mode reaches the frontend, and `s` triggers preview dismissal. Sending the
command as an undelayed X11 key burst reproduces it; constraining the editor,
renderer and fixture to one CPU also exposes the ordinary 25 ms typing case.

Expected behavior: the command leader and remaining command text reach the
editor in order, the horizontal split succeeds, and returning focus retains
preview navigation. Both standalone and persistent windows must behave this
way. Any queued input must preserve the attachment and frame physically seen
at capture time so delayed Enter cannot approve a newly presented prompt.
