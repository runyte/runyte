Finder workers read `.gitignore` and `.ignore` with `fs::read_to_string`
without validating their file type. An ignore file that is a named pipe with
no writer blocks the entire name or content scan indefinitely. Cancelling
the finder cannot interrupt that blocking open. The shared preview worker
also opens any path as an ordinary file, so a previously discovered file
replaced with a named pipe stalls later previews. Content reads check a
pathname's size before reopening it without a read limit; replacement or
growth can bypass the intended 4 MiB limit.

Reproduce the ignore case by creating a temporary project with an ordinary
text file and `.ignore` as a FIFO, then starting an ignore-aware Finder scan.
No terminal scan result arrives. Previewing a FIFO directly through
`FilePreview::from_path` or `FilePreview::snippet_from_path` also waits for a
writer instead of reporting an unreadable source.

Open regular files through a descriptor-validated, nonblocking boundary and
apply content limits to the bytes read, including snippet previews. Invalid
ignore files should count as skipped inputs while ordinary ignore rules and
regular-file symlink behavior remain supported. Bound ignore-file input to
4 MiB before parsing so malformed or growing metadata cannot consume an
unbounded worker allocation. Tests should run FIFO cases inside a bounded
subprocess and keep all filesystem/configuration paths in temporary storage.
