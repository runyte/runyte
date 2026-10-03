Git metadata readers can block indefinitely when an expected text file is a
named pipe. The direct workspace Git-facts reader opens `.git`, `commondir`,
`HEAD`, and `config` through `File::open` before checking the opened file's
type. Listing remembered workspaces can therefore stall on a FIFO with no
writer. The commit-network reader similarly opens the shallow-boundary file
directly, outside the Git subprocess cancellation and timeout machinery.

Create a temporary workspace with a `.git` directory, a normal `HEAD`, and a
FIFO at `.git/config`, then call `read_workspace_git_facts`. It waits for a
writer instead of returning unavailable remote information. The network
reader has the same failure when its resolved shallow-boundary path is a
FIFO; a controlled Git fixture isolates this direct read from Git's own
metadata access.

These readers should promptly reject non-regular files while retaining their
existing size bounds and ordinary-file behavior. The regular-file opening
check must apply to the opened descriptor, so replacement between an earlier
metadata check and `open` cannot reintroduce a blocking FIFO read. The merge
disk fingerprint reader also needs this guarantee after its initial regular
file check. This does not change the separately deferred filesystem-path
confinement design.
