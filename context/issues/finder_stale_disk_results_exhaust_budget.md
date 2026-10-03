Content Finder drops matches from unopened files when an earlier scanned file
is already open and its saved contents contain enough matches to exhaust the
50,000-row content budget. The scanner reads and counts those saved rows, then
the application discards them because the open buffer is authoritative. Rows
discarded at that later boundary still consume the scanner's budget.

Create `z.txt` with 50,000 lines containing `needle` and `a.txt` containing
`needle from disk`. Open `z.txt`, replace its contents with `changed in memory`
without saving, and open content Finder. The current reverse-name traversal
reads `z.txt` first. The stale saved matches consume the scanner limit and the
matching row in `a.txt` is omitted, even though no retained live result needs
that budget. The synchronous embedding path and the background scanner both
have the same ordering error.

The captured paths of authoritative open buffers must be excluded before disk
files are read or counted. Existing post-scan checks still need to protect
against buffers opened while a scan is running. The total candidate ceiling,
query revision guards, and cancellation behavior must remain unchanged.
