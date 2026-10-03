`WorkspaceHost::save_buffer` reports a successful buffer revision when the native
save workflow refuses the write or encounters an I/O error. Native saves report
these outcomes through editor feedback and normally return `Ok(())`; the host
adapter currently interprets that command-handling result as a completed write.

Reproduce by opening a file through a host, modifying its buffer, overwriting the
file externally, and calling `save_buffer`. The external contents remain intact
and the editor reports the stale-file conflict, but the host returns success.
A scratch buffer with no path and a clean generated read-only buffer also return
success without being written. Saving a commit-message buffer or an edited
directory can initiate asynchronous work or a native confirmation while the
caller immediately receives success.

The synchronous host API should return a revision only after an ordinary file
write completes. Refused and failed writes must return errors, and saves that
require asynchronous execution or native confirmation must be refused without
starting them. Native save-command feedback and confirmation behavior should
remain unchanged. A write that commits successfully with a durability warning
must retain its explicit successful-write outcome rather than being inferred
from feedback severity or the buffer's dirty flag.
