---
title: "Special-file document reads can stall the editor or its workers"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 06bc2b3
---

## Resolution

Commit `06bc2b3` (`fix(files): refuse special objects before document reads`).

`path_safety::open_regular_file` now admits an opened descriptor only if it
is a regular file and uses nonblocking Unix opens so a named pipe cannot wait
for a peer before validation. Buffer loading, reload, observation, conflict and
metadata checks, save permission probes, ACL copying, binary sniffing, and Git
local readers share this boundary. Symlinks to regular files remain supported.
If a save has already exchanged its temporary with a target that can no longer
be inspected, the displaced object follows the existing restoration path.

Two startup fixtures formerly used intentional FIFO blocking. Their replacements
verify presentation ordering for an ordinary document and prompt rejection of a
named pipe with restored terminal attributes. Unsupported special files are
now refused as documents rather than accidentally accepted as streams.

`special_file_io_refuses_promptly_and_preserves_documents` and its bounded
`special_file_io_fixture` in `src/buffer/tests/special_files.rs` cover direct
and symlinked pipes, device rejection, open/reload/observe/save behavior,
retained edits, and restoration after a target race. Both pass.
`document_open_presents_startup_before_the_document_frame` and
`named_pipe_startup_refuses_without_a_writer_and_restores_the_terminal` in
`tests/local_protocol.rs` pass through real PTY startup.

Known limitation: regular-file I/O on a stalled filesystem can still block.
Native macOS and Windows execution remains subject to their CI suites.

## Report

# Opening a named pipe can block document loading indefinitely

The ordinary document-open path probes a path with
`external_open::looks_binary`, which opens and reads it synchronously without
checking whether it is a regular file. On Unix a named pipe with no writer
blocks in `File::open`, stopping editor input. `read_text_and_state_limit`
uses nonblocking opening and a regular-file check only for explicitly bounded
reads; ordinary `Buffer::open` does neither. `observe_file` can likewise block
the file-monitor worker if an already monitored file is replaced by a pipe.

Create a named pipe in a temporary workspace and open it with `:open` or as a
launch target without starting a writer. The call cannot complete. Replacing
an observed regular file with a named pipe can stop subsequent observations
for unrelated buffers as well.

The same blocking open is used for file metadata hints, save conflict checks,
the save write-permission probe, and Linux ACL copying. Git comparison reads
open a working-tree path directly; bounded Git counts, fingerprints, and
repository-marker reads check the pathname first but can still encounter a
replacement pipe when opening it. These paths must share descriptor admission.

When a concurrently replaced save destination is exchanged into the backup
path, failure to inspect that displaced object currently returns before the
existing recovery exchange. Refusing unsupported descriptors must restore the
displaced object and retain the complete replacement contents rather than leave
the temporary save installed after reporting failure.

Document reads and binary probes must refuse unsupported special files
promptly, preserving the existing document and input responsiveness. Regular
files, symlinks to regular files, new-file creation, and bounded document reads
must retain their current behavior. The opened descriptor needs validation so
that a pathname replacement between inspection and opening cannot reintroduce
the blocking read. Tests must use temporary storage and explicitly bounded
fixture lifetimes.

The same descriptor admission problem affects disk conflict inspection,
metadata polling, save permission checks, ACL copying, and Git readers when a
regular pathname is replaced after their preliminary metadata check. A save
that has already exchanged the target with its temporary must restore a
displaced unsupported object when inspection fails. Returning immediately
would leave the replacement installed after reporting failure.
