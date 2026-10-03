`--session-list` writes workspace names and directory paths directly into its
terminal table. A valid Unix directory containing a newline creates additional
rows, and a directory containing an escape character can emit terminal control
sequences. Width calculation also treats these bytes as presentation content,
so one filesystem name can corrupt the rest of the listing.

The session table must show each workspace on exactly one row and render
control characters as visible escaped text. Ordinary Unicode names and paths
must remain readable, and raw workspace identities used for selecting and
connecting must remain unchanged. A regression can construct a workspace row
whose directory contains newline, tab, carriage return, and ESC, then inspect
the formatted table without starting a terminal or creating such a workspace.
