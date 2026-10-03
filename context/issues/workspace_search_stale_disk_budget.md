Workspace search can omit valid results when an open file has unsaved changes
that remove matches. The disk traversal applies the 10,000-result cap before
the worker replaces matches from open files with their captured buffer text.
Matches that no longer exist in the buffer can exhaust the disk budget and
stop traversal before unopened files are read. Removing those stale matches
afterward does not resume the traversal.

For example, create `a.txt` containing one `needle` line and `z.txt` containing
10,000 `needle` lines. Open `z.txt`, remove all its text without saving, and
run workspace search for `needle`. The reverse entry traversal fills its
budget from the on-disk `z.txt`, then removes those matches because its open
buffer is empty. The result contains no matches even though `a.txt` still
matches.

Captured open-buffer text must be authoritative before any result budget is
spent. Unopened files must remain searchable up to the existing result limit;
the limit, cancellation behavior, and generated result-buffer UX remain intact.
