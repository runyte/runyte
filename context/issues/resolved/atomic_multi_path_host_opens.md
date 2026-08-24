---
title: "A multi-path host open leaves earlier buffers open when a later path fails"
status: resolved
reported: 2026-08-14
resolved: 2026-08-15
legacy_commit: a89e3c9
---

## Resolution

Commit a89e3c9 (Open multi-path host requests atomically) replaced the sequential open loops behind `OpenBuffers` and wait-request creation with a single prepare-then-commit path, `App::host_open_files`.

The defect was in the loop shape rather than in any one function. `main.rs` mapped `host.open_buffer` over the request's paths and collected into `Result<Vec<_>>`, and `WorkspaceHost::create_wait_request` ran the same open call in a `for` loop before allocating its token. Both opened each path fully — reading the file, pushing the buffer, tracking it in Git, notifying the language server, and for the first path retargeting the pane — before looking at the next one. The first error short-circuited the collection, so every path already opened stayed open while the client received only an error, with no buffer ids and no wait token to name that state with. The buffers were unreachable to the client and, in the wait case, accounted for by nothing.

`host_open_files` splits that into two phases. The first resolves each path and builds its buffer with the same `open_or_new` and `parse_buffer` the interactive editor uses, holding the results in a local `staged` vector and touching no editor state; a path already open contributes nothing, and a path repeated inside one request shares the entry the first occurrence staged. The second phase pushes the staged buffers, tracks them in Git, and touches the language server, none of which can fail on the request's own paths. Building the buffer is what validates the path, so there is no separate preflight predicate that could drift away from what opening actually requires — the shape follows the staged-commit pattern c39cac0 established for LSP workspace edits.

Activation is applied last, after the buffers are live, so the pane moves only for a request that has already succeeded. It still routes through `open_file` rather than a bare pane retarget: by then the buffer exists, so a file takes the already-open branch, and a directory keeps going through the explorer's own orphan adoption in `retarget_pane_directory`, which owns `pane.directory_buffer` and would be wrong to bypass.

`host_open_file` is now a single-path call into `host_open_files`, so both protocol entry points share one implementation. `buffer_for_path` was deleted; the new path was its only remaining caller.

A follow-up change made the explorer side of activation atomic as well. `retarget_pane_directory` read the listing after the pane had already adopted the buffer: it set `active_mut().directory_buffer` and only then called `reload_directory_buffer`, so a directory that could not be listed on entry left the pane owning an explorer it never entered. `pane_directory_buffer` had the same shape one level down, pushing a newly read directory buffer into the editor before its caller could fail. It now returns a `PaneDirectory`, either an index the editor already holds or an unpushed `Buffer`, and `retarget_pane_directory` does every read before the pane commits to anything: the new-buffer branch reloads the listing while it is still a local value and adopts it afterwards, and the entering branch reloads before assigning `directory_buffer`. `reload_directory_buffer` was split so its bookkeeping tail, `settle_reloaded_directory`, can run for a listing that was read before its buffer joined the editor. The two buffer-level reads it depends on, `Buffer::reload_directory` and `Buffer::retarget_directory`, already performed their fallible read before assigning any field, so they needed no change.

That defect was reachable from the interactive editor and not only through the protocol: stepping off an explorer and re-entering its directory after it became unreadable left the pane holding it.

Tests in src/workspace/host.rs cover the behavior:

- a_failing_later_path_opens_no_buffer_and_moves_no_pane
- a_failing_later_path_allocates_no_wait_token

Tests in src/app.rs cover the explorer side:

- a_directory_that_cannot_be_listed_adopts_no_explorer

Both use a valid first file and a binary second file, and assert against the host's live buffer paths rather than the response, since the reported symptom is state the response cannot describe. The first also asserts the active pane did not move and that the valid path still opens afterwards; the second asserts no wait token or ordering entry survives. Both were confirmed to fail against the previous sequential behavior before the fix was restored.

A later correction removed the remaining directory window and an identity mismatch it exposed. An active directory is now entered after every request path has been prepared but before staged buffers are committed. The explorer's actual pane-owned buffer id replaces the prepared directory slot, so a retained explorer can be retargeted without leaving the separately staged directory live or making a wait token own a buffer other than the one on screen. Repeated occurrences receive that same id, including when the path was already open in another explorer. If the pane's reusable explorer has unsaved edits to a different directory, the host request is rejected before commit rather than creating a wait request whose activation depends on a later interactive confirmation.

Tests in src/workspace/host.rs cover the correction:

- an_activated_directory_wait_uses_the_panes_reused_explorer
- an_already_open_repeated_directory_wait_uses_the_activated_explorer
- a_dirty_reused_explorer_rejects_directory_activation_before_commit

## Report

Opening several paths through the local protocol was not atomic when a later path failed.

`OpenBuffers` opened paths sequentially and returned their IDs only after the whole list succeeded. Wait-request creation likewise opened every path before allocating and inserting the wait token. If the first path was valid and a later path was binary, unreadable, or otherwise invalid, the earlier buffers remained open and could become active, but the client received only an error and no buffer IDs or wait token with which to account for that partial state.

All paths needed to be preflighted before host state changed, or newly opened buffers and the previous activation state needed to be recorded and rolled back on failure. Token allocation and activation needed to occur only when the complete request could commit. Protocol tests needed to use a valid first file and a failing second file for both `OpenBuffers` and wait creation.

Relevant code was src/main.rs in `OpenBuffers` handling and src/workspace/host.rs in `create_wait_request`.
