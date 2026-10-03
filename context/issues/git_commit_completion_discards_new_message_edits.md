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
