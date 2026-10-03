Path completion can retain an obsolete directory listing indefinitely on
Windows after another directory replaces it at the same pathname. The listing
cache checks the filesystem object identity only on Unix. On Windows it stores
no identity, but still treats an old, unchanged modification time as sufficient
to reuse a listing without an expiry.

Reproduction: create two directories with different entries and give both the
same modification time older than two seconds. Complete a path inside the first
directory so its listing is cached. Rename it aside, then rename the second
directory onto the first pathname. Completion continues offering the removed
directory's entries rather than the replacement's entries.

Windows directory identity must participate in cache invalidation. If identity
cannot be obtained, the existing short reuse window must bound stale results;
an unchanged timestamp alone must not grant unlimited reuse. Ordinary unchanged
directories should continue using the cache instead of being read each time a
key or redraw requests completion.
