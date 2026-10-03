Each Finder ranking and preview worker owns a strong `Arc` to its mailbox.
The mailbox also owns the sender for the channel on which that worker waits.
Dropping the final `FileScanner` handle therefore never disconnects the
worker's receiver: the idle thread itself keeps its sender alive. The worker,
its retained candidate state and its event sender survive the editor instance
that created them. Closing the picker clears ranking state but does not end
the worker, so ordinary workspace/editor teardown can accumulate threads.

Worker ownership must allow the final scanner handle to release the mailbox
and disconnect its wake channel. Scanner clones should keep workers usable
until the final clone is dropped. Regression coverage should start and drain
both worker kinds, verify a surviving clone still works, then drop all
scanner handles and observe mailbox release and event-channel disconnection
within a bounded deadline.
