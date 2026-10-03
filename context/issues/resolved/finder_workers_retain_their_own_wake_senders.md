---
title: "Finder workers retain their own wake-channel senders"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 1070431
---

## Resolution

Commit `1070431` (`fix(finder): release worker mailboxes with the final scanner`).

Ranking and preview workers now hold weak mailbox references and upgrade only while extracting queued work. They release that temporary owner before processing or waiting again. The final scanner clone can therefore drop the mailbox sender, disconnect the worker wake channel, and release retained worker state and event senders.

Coverage: dropping_the_final_scanner_releases_idle_rank_and_preview_workers in src/file_picker.rs starts both workers, verifies another scanner clone remains usable, then checks event-channel disconnection and mailbox release after the final clone drops. It timed out before the change; all 53 selected file-picker tests pass.

## Report

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
