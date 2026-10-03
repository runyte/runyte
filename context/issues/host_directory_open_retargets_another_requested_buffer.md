A multi-directory host open can return the wrong directory buffer for a
successfully accepted request. If the active pane's explorer already shows
`second`, opening `[first, second]` with activation enabled prepares `second`
as an existing live buffer. Activating `first` then reuses that explorer and
changes its identity to `first`. Both returned buffer IDs consequently refer
to `first`, and `second` is no longer open.

Every returned buffer ID must continue to identify its corresponding requested
path. Explorer reuse must preserve live buffers that the same request names
under a different directory identity. Ordinary single-directory navigation
should continue to reuse the pane's explorer, and a failed directory read must
publish no partial state.
When both requested directories already have live explorers, an unclaimed
target should be adopted instead of allocating a duplicate; an explorer shown
or reserved by another pane must retain that pane's ownership.

Reproduce by creating two directories, constructing an app with `second` open,
and calling `host_open_files(vec![first, second], true)`. The second returned
buffer currently has path `first`, and both IDs are equal. The failure needs no
concurrent filesystem changes.

Repeated aliases of an already-open directory have a related mismatch. With
the pane showing `second` and `first` open in the background, requesting
`[first, alias_to_first, first]` prepares one canonical identity but returns
different IDs: final activation replaces entries by their raw requested paths,
so the alias still refers to the previous background explorer. All occurrences
of the activated directory identity must receive the pane's activated buffer
ID, as repeated identical paths already do.
