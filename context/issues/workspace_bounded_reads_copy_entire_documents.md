Bounded persistent-session reads perform work proportional to the whole
document before returning their bounded output. `WorkspaceHost::read_buffer`
first calls `Buffer::to_string`, copying every byte, and only then retains the
first 1 MiB. Buffer rows in `WorkspaceHost::session_preview` similarly call
`line_string` before retaining the first 240 characters of each of at most eight
rows. A large single-line document therefore causes a large temporary
allocation even for a compact preview.

These synchronous operations run on the workspace host thread. Repeated
control reads or session previews can delay editor input and rendering, with
temporary memory and copy cost growing with the source document rather than
the response limit.

Read only the required rope prefix or line slice before converting it to an
owned string. Preserve the exact 1 MiB UTF-8 boundary and `truncated` flag for
buffer reads, and the current 240-character preview limit, terminal-control
replacement, tabs, line terminator treatment, and viewport position.

Reproduction: open a document containing one 128 MiB ASCII line. Request its
buffer contents through the persistent-session control API or request a
session preview. Although the result contains at most 1 MiB or 240 characters,
respectively, the current implementation first allocates and copies the whole
128 MiB line. Compare the same requests against a small document with an
identical returned prefix; only the discarded suffix changes the copy cost.
