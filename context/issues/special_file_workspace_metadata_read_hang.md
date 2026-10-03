A Unix persistent session can hang while reading endpoint metadata or its stored
session name if the corresponding runtime file is replaced by a named pipe.
The endpoint permission checks accept an owner-private FIFO with mode `0600`,
and `read_bounded_file` opens it with blocking `File::open`. Without a writer,
that open never completes. The read byte limit does not bound the open.

Endpoint discovery, liveness checks, and stored session name loading should
reject non-regular files promptly while retaining the existing private-file
checks, size limits, and normal metadata/name behavior. The opened descriptor
must be checked so a file replacement between the path check and open cannot
turn the read into an unbounded FIFO wait.

Reproduction: in a temporary workspace, create the normal private endpoint
directory and replace `endpoint.json` (the path returned by
`LocalEndpoint::metadata`) with a mode-`0600` FIFO. Calling the endpoint's
recorded-host liveness check blocks indefinitely. A FIFO at the stored session
name path produces the same result when loading the name. A bounded subprocess
fixture can exercise both paths without leaving a blocked test thread behind.
