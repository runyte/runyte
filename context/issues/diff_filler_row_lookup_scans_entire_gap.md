`DiffSession::row_at_or_above` searches backward through every aligned row until
it reaches a real row of the requested side. Each candidate performs a binary
search through the alignment runs. A large inserted or deleted block therefore
requires work proportional to the entire filler gap on every viewport mapping,
even though the run already records the preceding document row.

For example, compare `head\ntail\n` against `head\n` followed by one million
`x\n` lines and `tail\n`. Both inputs fit the 4 MiB comparison limit. Mapping
the left side near the end of the inserted block scans one million filler
positions to return row zero. Linked diff viewport preparation invokes this
mapping on the editor thread, so moving through large one-sided changes can
delay rendering.

Find the containing run directly and use its side range to return the nearest
real row. Preserve actual rows, leading filler with no preceding row, unequal
replacement runs, empty sides, and the existing virtual rows past the alignment.
The result must match the backward scan without visiting each filler position.
