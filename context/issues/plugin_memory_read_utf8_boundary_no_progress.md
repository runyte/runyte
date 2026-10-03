The shipped memory-provider example can acknowledge a resource read with an
empty string and `eof: false` when the requested byte limit cannot hold the
next UTF-8 scalar. Its end-boundary adjustment also walks backward past the
requested offset when that offset itself splits a scalar. The resulting empty
slice can conceal the invalid starting offset.

For a resource containing `é猫`, reads at offset 0 with limit 1, offset 2 with
limit 2, or offset 1 with limit 1 return empty non-final chunks. These reads
must return `invalid_argument`, preserving the requested byte coordinates.
Valid scalar-aligned reads must still return complete characters, and an empty
read at the actual end of the resource must remain a successful EOF response.

The transport-neutral remote provider already enforces these boundaries; the
memory example should demonstrate the same public resource-read contract in
both conditional and confirmed best-effort modes.
