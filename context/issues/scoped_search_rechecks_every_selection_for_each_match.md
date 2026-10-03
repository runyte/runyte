`text_matches` checks every candidate search match against the region list with
`spans.iter().any(...)`. A search started over thousands of selections therefore
performs quadratic containment work when the number of matches scales with the
number of regions. Both live search previews and accepted searches use this
path, so narrowing the results of a broad earlier search can stall input.

Reproduce with a buffer containing 20,000 copies of `aa `, select each `aa`, and
search for `a` inside the selections. The 40,000 candidate matches repeatedly
scan the region prefix even though matches and normal selections are ordered.

Search containment should advance through region boundaries once while matching
in document order. Preserve the requirement that one individual region contain
the entire match; joining adjacent or overlapping regions must not allow a match
that no single original selection contained. Preserve Unicode offsets,
zero-width regular-expression matches, and empty-region behavior.
