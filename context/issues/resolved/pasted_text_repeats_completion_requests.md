---
title: "Pasted text repeats completion requests at the final caret"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: bd64610
---

## Resolution

Commit `bd64610` (`perf(completion): refresh pasted text at its final caret once`).

after_insert previously iterated over characters after the entire insertion was already applied, repeatedly querying the same final caret. It now handles the complete insertion once, uses its last character for service triggers, and updates popup filtering with the whole inserted string. Explicit sessions terminate on whitespace and pasted closing delimiters dismiss stale signature help.

Coverage: pasted_text_requests_language_help_only_at_the_final_caret, pasted_text_refreshes_an_explicit_completion_once_and_keeps_its_filter, and pasted_text_filters_existing_automatic_completion_with_the_whole_insertion in src/app/tests/pasted_completion.rs. Recording service queues reproduced 41 automatic requests and 20 explicit refreshes before the fix; each insertion now refreshes once. All three regressions and 102 language workflow tests pass.

## Report

Pasting text refreshes completion and signature help once per pasted character
after the complete insertion has already moved the caret to its final position.
Every refresh therefore observes the same final buffer and caret, and trigger
characters in the middle of the pasted text issue requests for a context that
is no longer at the caret. Path completion can also repeatedly resolve and
rank the same final directory listing on the editor input thread.

With a language server advertising `.` as a completion trigger and `(` as a
signature-help trigger, paste `.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.(.` in
Insert mode. A single insertion sends 41 requests: 21 completions and 20
signature-help requests, all at the same final position. With explicit language
completion active, pasting 20 consecutive `.` characters restarts it 20 times.

A text insertion must refresh the final completion context once, with at most
one request for each applicable language service. Ordinary single-character
typing, explicit completion ownership, filtering by the complete inserted
text, whitespace termination, and path/word completion precedence must remain
intact. Trigger characters that are no longer at the caret must not schedule
obsolete requests.
