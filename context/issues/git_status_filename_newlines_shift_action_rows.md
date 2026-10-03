The changed-file list interpolates raw path display text into each row.
Filenames containing newlines produce multiple buffer rows, while the
`status_entries` identity mapping still records one row per file. A selection
on the additional row can therefore stage, unstage, open, or prepare to discard
a different file from the one displayed. Later files can become unreachable
through their visible rows. Rename source names have the same problem.

Reproduce using a modified path named `first\ncontinuation.txt` followed by
another modified file. Build the Git status projection, join its row text
with newlines, and compare buffer row numbers with the corresponding entries.
The newline in the first path shifts visible text away from its identity.

Every status entry must occupy exactly one logical row. Escape control
characters in display names with the existing Git path-display helper while
preserving original operating-system paths for every action. Both sides of
rename labels need the same treatment; ordinary names must keep their current
presentation.
