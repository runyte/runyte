---
title: "A one-character search shows only the primary match as selected"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: f209e1f
---

## Resolution

Commit `f209e1f` (`Draw every match of a one-character search`) fixed the
drawing of the search result, not the search itself: every match was already
selected, and `n`, `N`, and a batch edit over the multi-selection all acted on
all of them. Only the picture was wrong.

`snapshot_text_runs`'s `role_at` in `src/snapshot.rs` decides what each
character is: a caret, selected text, or plain. A committed search sets
`search_selection`, and while that presentation is pristine the secondary
carets are deliberately suppressed so the result reads as a set of matches
rather than as a set of cursors; the whole of each match is painted as
selected instead. That painting walks the ranges and skips the empty ones,
because an empty range — anchor and head on the same offset — is a bare caret
that covers no text. A match one character long is exactly such a range:
`buffer_matches` returns `Range::new(start, end - 1)`, so a single character
gives `start == end - 1`. Its caret was suppressed with the other secondary
carets and its span was skipped as empty, leaving it plain. The primary match
still drew, through the earlier `PrimaryCaret` branch, which is why one `s`
was visible and the rest were not. Pressing Esc left Select mode, which made
the search presentation no longer pristine and brought all the carets back —
the moment the matches became visible.

`role_at` now paints a pristine search's empty ranges as selected, gated on
`SelectionSemantics::Runyte`, where a range covers the character it sits on;
under half-open semantics an empty range genuinely covers nothing. The skip
in the loop below is unchanged, so a bare caret outside a search still
selects nothing.

Covered by `a_single_character_search_draws_every_match_and_not_only_the_primary`
in `src/snapshot.rs`, alongside the existing
`pristine_search_hides_secondary_carets_until_the_selection_moves`.

Known limitation: a regular expression that matches zero characters, such as
`x*`, also produces an empty range, and one of those is now painted over the
character following it rather than left invisible. The two cases are
indistinguishable once a match has become a range.

## Report

Searching for a single character — `s a Enter` — found every `s` in the text
but appeared to select only the primary one. Pressing Esc afterwards made all
of the matches visible. Patterns of two characters or more behaved correctly.
All matches should be visible immediately.
