---
title: "A repeated --wait request could show a stale Git merge message"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: c246cac
---

## Resolution

Fixed in `c246cac` "Refresh reused wait buffers from disk".

`App::host_open_files_with_refresh` used to treat a live buffer with the same
path as the complete answer to every host open. That is correct for an ordinary
open and for unsaved shared editor state, but it was wrong at the start of a
new editor-wait workflow. A completed `--wait` deliberately leaves its clean
buffer in the persistent host, while Git rewrites `.git/MERGE_MSG` before the
next merge. `WorkspaceHost::create_wait_request` therefore attached the new
request to the old clean text without reading Git's new file.

The buffer's recorded disk state still described that old text. After the
stale branch name was edited, the normal external-change guard correctly
refused to overwrite Git's newer merge message. The buffer consequently
remained dirty, and quitting the attached TUI could not complete a wait request
whose modified buffer had not been saved, producing the reported terminal
error.

`WorkspaceHost::create_wait_request` now distinguishes buffers owned by an
existing pending wait and opens new wait requests through
`App::host_open_wait_files`. That path stages a reload of every reused clean
file before mutating live state, then replaces its text under the same buffer
identity with a new global revision and resynchronizes its syntax, word index,
language service, guards, and pane positions. Preloading makes a multi-path
request atomic: if any later path is invalid, none of the clean reused buffers
is refreshed. Dirty buffers and buffers belonging to a pending wait retain
their in-memory text rather than being replaced.

Tests:

- `a_later_wait_refreshes_a_clean_reused_file_from_disk` in
  `src/workspace/host.rs` reproduces consecutive `MERGE_MSG` waits, checks the
  new text and revision, and saves an edit without an external-change refusal.
- `a_new_wait_never_refreshes_dirty_or_pending_buffer_text` in
  `src/workspace/host.rs` protects unsaved work and concurrent wait ownership.
- `a_failing_later_path_allocates_no_wait_token` in `src/workspace/host.rs`
  verifies that a failed multi-path wait leaves an already-open clean buffer's
  text and revision untouched.

## Report

Running `git merge dev` with Git configured to use `runyte --wait` opened a
message saying that the `security` branch was being merged instead of `dev`.
Changing the branch name and entering `:wq` did not allow the message to be
saved. After `:q!`, Git reported:

```
hint: Waiting for your editor to close the file... Error: attached TUI quit before successful wait completion: modified wait buffers must be saved before completing the request
error: there was a problem with the editor 'runyte --wait'
Not committing merge; use 'git commit' to complete the merge.
```

The unfinished merge then prevented another `git merge dev` because
`MERGE_HEAD` still existed. The merge needed to be completed, and Runyte needed
to open the current merge message and allow it to be written.
