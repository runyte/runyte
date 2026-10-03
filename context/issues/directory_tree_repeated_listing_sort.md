Directory-tree navigation and presentation clone and sort every expanded
directory listing each time visible rows are requested. The background reader
already sorts those listings. Repeated row requests from cursor movement,
selection reconciliation, and redraw therefore duplicate path allocations and
repeat comparisons even when the filesystem has not changed.

The work increases with all expanded entries, rather than only entries visible
in the pane. A tree can retain 512 directory listings with up to 4,096 entries
each and expose up to 100,000 rows, so the extra sorting and cloning can become a
material input and redraw cost in a large workspace.

Maintain the existing directories-first, filename-sorted ordering when listings
change, including immediately published filesystem operations. Construct visible
rows by borrowing the already ordered entries. Navigation order, dotfile
visibility, and background refresh behavior must stay unchanged.
