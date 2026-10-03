---
title: "Git commit completion discards newer message edits"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 0b0119b
---

## Resolution

Commit `0b0119b` (`fix(git): preserve message edits made after commit submission`).

Ordinary asynchronous completion searched for whichever commit-message buffer
was currently live. Reviewed merges retained a buffer identity but did not
compare its submitted revision. Both now capture request ID, buffer identity,
revision, originating pane binding, and return location at submission. Success
retires only that unchanged buffer; edited or replacement messages survive.
Returning to the original view also requires the original active pane binding,
so a late result cannot redirect unrelated work. Failure releases the capture
and keeps the message for retry. A completed reviewed merge drops its spent
approval while retaining any later text and reporting that preservation.

Coverage: `asynchronous_commit_completion_preserves_later_message_edits_and_view_ownership`
in `src/app/tests/git.rs` covers unchanged, edited, abandoned/reopened,
navigated, mutation-failed and worker-failed requests.
`merge_ui_submitted_commit_survives_close_attempts_and_detach_until_result`
in `src/app/tests/git_merges.rs` additionally edits a submitted reviewed merge
message. Lifecycle fixtures in `src/app/tests/git_merge_lifecycle.rs` now deliver
the captured request ID. Both data-loss cases failed before the fix; all 182
app Git tests passed afterward. Root independently reviewed the implementation.

## Report

An ordinary asynchronous Git commit captures the submitted message text but
does not capture the originating message buffer or its revision. When the
worker reports success, `apply_git_mutation_result_for_request` clears and closes the first live
commit-message buffer. Edits made after submission can therefore be discarded
even though Git never received them. If the original message is abandoned and
another message is opened while a hook runs, the old completion can close the
new message instead. Completing a commit after navigating elsewhere can also
retarget the active pane through the global commit return location.

Reviewed merge commits retain their originating buffer and prevent closing it
while the worker runs, but the buffer still accepts text edits. Their successful
completion also clears the current text without checking the submitted revision.
After approving a merge message, submit its save, edit the message while the
commit is pending, and deliver the successful result: the later text is lost.

Reproduce with a recorded or delayed Git service: open an ordinary commit
message, enter text, submit its save, edit the message before delivering the
successful completion, then deliver the result. The new text disappears.
Repeat by abandoning the original buffer and opening a different message
before completing the original request; the replacement must remain intact.

A completion may close only the exact unchanged message buffer submitted by
that request. Later edits, replacement buffers, and unrelated active panes
must survive both successful and failed responses. Normal successful commits
must still close their unchanged message and return to the original view;
failure must keep the submitted message available for retry. This work does
not change which index content Git commits or the reviewed merge authority.
