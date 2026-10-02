Exited terminal sessions remain in `Space t t` and Finder results. By
default, exiting a terminal (for example, by typing `exit`) should forget
the terminal session and remove it from both lists. The retention behavior
should be configurable through `Space o o`.

To reproduce, start a terminal, type `exit`, then open `Space t t` or the
Finder. The exited terminal is still listed.
