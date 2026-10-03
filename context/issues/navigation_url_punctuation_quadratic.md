Inferring a web link under the cursor repeatedly counts opening and closing
delimiters across the entire remaining URL while trimming unmatched trailing
parentheses, brackets or braces. In `navigation_target::under_cursor`, each
trailing `)` invokes two `str::matches(...).count()` scans before removing one
character. A row containing `https://example.test/` followed by many unmatched
closing parentheses consequently takes quadratic time when `gf` infers the
target on the editor thread. Terminal review uses the same inference helper.

The punctuation trimming should preserve its current treatment of balanced
URL delimiters and prose punctuation while scanning each candidate a bounded
number of times. Regression coverage should include long unmatched suffixes,
balanced and nested delimiters, mixed punctuation, and a cursor outside the
retained URL. A stress comparison should measure the actual helper with
increasing suffix lengths rather than using a wall-clock assertion in a test.
