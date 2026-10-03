The local file-manager example retains a newly allocated host directory handle
when a later listing page fails. `listing` accumulates pages without releasing
the handle on error, and `refresh` starts its cleanup only after `listing`
returns successfully.

If the directory changes between pages, the host returns `stale`. Its first
page's snapshot remains allocated even though the example never adopts or
displays it. With one directory already visible, this consumes the second and
last directory slot, causing attempts to browse another directory to fail with
`Directory snapshot limit reached; release an old directory`.

On a failed paginated listing, release any newly acquired directory handle
while preserving the handle backing the currently displayed directory. Preserve
the original listing error if cleanup also fails. Reproduce by receiving a
first page with a new directory handle and a non-null `next`, then returning
`stale` for the subsequent page.
