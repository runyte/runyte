Search scoping treats every selection as an inclusive Runyte range. Syntax
expansion creates half-open selections, whose ending offset is outside the
selected text. `pane_scoping_region` and `region_spans` apply `operative_span`
without preserving that distinction, so the search includes the next character.

For example, expand the selection over `demo` in `fn demo() {}` and search for
`(`. The search preview and accepted search can match the unselected opening
parenthesis. A one-character half-open syntax selection can also be mistaken
for a two-character region instead of retaining whole-buffer search behavior.

Search preview, accepted searches, and repeats after edits should use the exact
selected character spans. Keep the existing threshold of at least two selected
characters and retain the selection's boundary semantics when storing and
mapping the search region through text changes.
