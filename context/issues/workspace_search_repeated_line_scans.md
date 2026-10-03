Workspace search repeatedly scans the same text while producing several
matches from one line. For every match, `extend_matches` counts characters
from the beginning of the line to the match and trims the complete line again
to build its 240-character preview.

A long line containing many matches therefore incurs quadratic prefix-count
work. Trailing whitespace can independently cause the same suffix to be scanned
up to 10,001 times, the worker's result-limit sentinel included. This can delay
search results and cancellation even though scanning runs off the input thread.

For reproduction, search for `λ` in a line containing 10,000 `λ` characters
followed by one MiB of spaces. All matches share the same preview. Character
columns and match lengths must remain correct for Unicode and zero-width
regular expressions. Compute columns from successive match boundaries and
construct the shared line preview once, retaining the existing result cap and
snapshot format.
