---
title: "Tree-sitter reparsing blocked keystrokes and removed highlighting from large documents"
status: resolved
reported: 2026-08-17
resolved: 2026-08-18
legacy_commit: 00227eb
---

## Resolution

Commit `00227eb` (`Move syntax reparsing off the input path`) moved incremental
Tree-sitter reparsing out of `App::reparse` and into a background worker. The
old function called `DocumentSyntax::update` on the editor thread, so its cost
was paid by the keystroke; `App::park_reparse` avoided that cost only by moving
the tree out of `App::syntax`, which made both highlighting and structural
features disappear until `App::flush_parked_syntax` ran.

The replacement clones the current tree into a coalescing worker request and
retains the original on the editor thread. A watch channel gives parser work a
queue depth of one, and the worker reduces every typing burst to one change
from the retained parsed text to the newest supplied text. Finished trees
return as `SyntaxEvent`s and are applied only by the standalone or attached
workspace-host event loop. Both the retained tree's `SyntaxRevision` and the
target text revision must still match, so a late result is discarded.

`StaleSyntax` enforces the stale-tree boundary. It privately owns the retained
`DocumentSyntax`, parsed text, and pending-edit log, and exposes only a
`TranslatedSpans` result. Viewport offsets are mapped backward through inverse
transactions, `DocumentSyntax::spans` runs against the text the retained tree
actually describes, and its results are mapped forward. No structural method
can obtain that tree: `App::syntax` is `None` while parsing, so folds, outline,
matching brackets, enclosing delimiters, text objects, and structural
selection continue to return no result until the current tree is drained.

The 50,000-line/1 MB defer band, the 200,000-line/8 MB refusal, the parked
syntax timer, and the `syntax highlighting off: file too big` status were
removed. The parse-dispatch seam remains inline until a worker is attached, so
the established synchronous editor and syntax tests stay deterministic while
focused tests opt into the production path.

Tests covering the behavior are:

- `stale_tree_exposes_translated_spans_but_no_structure_until_drain` and
  `late_tree_is_rejected_and_the_latest_coalesced_revision_applies` in
  `tests/background_syntax.rs`;
- `typing_into_a_large_highlighted_file_keeps_colours_during_reparse`,
  `minified_documents_reparse_in_background_past_the_old_byte_limit`,
  `a_discrete_edit_in_a_large_highlighted_file_stays_under_its_ceiling`, and
  `syntax_highlighting_continues_past_the_old_line_limit` in
  `tests/performance.rs`.

Known limitation: language injections are still dropped above
`INJECTION_LIMIT_BYTES` (128 KB), and `PARSE_TIMEOUT` remains five seconds.
Both policies were deliberately left for separate follow-up work.

## Report

Tree-sitter parsing and reparsing ran synchronously on the keystroke path, and
their cost followed the size of the document rather than the size of the edit.
Every limit Runyte placed on highlighting existed to work around that one fact,
and each took highlighting away from a document that could otherwise have had
it.

The reparse after an edit was incremental in the sense that the old tree was
reused, but a flat document gives the root node one child per element, so an
edit anywhere rebuilds that list. Measured on generated JSON of the shape
`{"id": N, "name": "item-N"},` one line per element, a single keystroke cost
roughly:

```text
 25,000 rows (  952 KB):   5.7 ms
 50,000 rows ( 1.93 MB):  17.6 ms
100,000 rows ( 3.88 MB):  46.2 ms
200,000 rows ( 7.98 MB):  95.7 ms
```

A parse from scratch was about five times the incremental figure, so the same
8 MB document took about half a second to open. The relationship was close
enough to linear that the cost at a million rows was predictable and unusable.

Three workarounds were in place. Language injections had been dropped above
128 KB because `Syntax::update` re-runs the injection query across the whole
document on every edit (`INJECTION_LIMIT_BYTES` in `src/syntax/mod.rs`). Commit
`e690320` added two more: above 50,000 lines or 1 MB the reparse was parked and
ran once typing stopped, and above 200,000 lines or 8 MB the document was not
parsed at all and the status line reported
`syntax highlighting off: file too big`.

Each workaround had a visible cost. A document past the upper limit was never
highlighted, however long it stayed open and however little it was edited. A
document in the deferred band was shown as plain text for the length of every
typing burst, and its colours returned about 200 ms after the last keystroke.
While a document was parked its structural features were unavailable too —
outline, folds, matching brackets, and structural selection all read the same
tree — which was why only Insert mode deferred and leaving it flushed.

The parked tree had to be hidden rather than merely marked stale.
`tree_house::Syntax::update` applies the tree edits and the reparse in a single
call, so there was no way to shift a tree's positions cheaply and defer only
the parse. A tree that had not been reparsed described text the buffer no
longer held, and querying it would answer with offsets that no longer pointed
where they claimed. `App::park_reparse` therefore moved the tree out of
`App::syntax` entirely, so every existing reader saw a buffer with no tree, a
state they already handled.

The requested fix was to take parsing off the keystroke path rather than make
it cheaper in place. `src/lsp/` already provided the intended shape: all work
runs in one Tokio task while the editor holds a non-blocking handle and drains
events from the main loop, so no language server can stall rendering or input.
Syntax needed to work the same way: an edit hands the parser the text and
returns, and a finished tree arrives as an event the editor applies between
frames.

That change needed to remove the 200,000-line and 8 MB refusals, since a slow
parse would no longer be a slow keystroke, and stop the deferred band from
showing plain text by retaining the previous tree until the new one landed.
The injection limit and five-second `PARSE_TIMEOUT` were related follow-up
questions rather than part of this implementation.

Two properties had to survive. Snapshots had to stay deterministic and
frontend-independent, so a tree arriving mid-frame could not change what a
frame already being prepared would show. A result computed against one
revision could not be applied to another: `DocumentSyntax` already tagged its
results with a `SyntaxRevision`, so a late tree needed to be discarded against
the current revision rather than applied blindly. A document edited faster
than it could be parsed needed to degrade to a queue depth of one rather than
an unbounded backlog.

Relevant code was `src/syntax/mod.rs` for `DocumentSyntax`, `PARSE_TIMEOUT`,
and the `SYNTAX_*` limits; `src/app.rs` for `parse_buffer`, `App::reparse`,
`App::park_reparse`, and `App::flush_parked_syntax`; and `src/lsp/mod.rs` and
`src/lsp/transport.rs` for the task-and-handle pattern. The budgets in
`tests/performance.rs` covered the behavior, and
`a_discrete_edit_in_a_large_highlighted_file_stays_under_its_ceiling` held the
200 ms ceiling that the change needed to lower.
