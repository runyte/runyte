---
title: "Completed commands leave only an unfinished key prefix in the message row"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: e5069f7
---

## Resolution

Commit e5069f7 (`Show completed commands in the message row`) added completed-command feedback to the bottom message row. `RunyteGrammar::translate_modal` cleared its pending sequence when a binding resolved, and `App::snapshot` could consequently show only the earlier incomplete prefix or whatever status text the command happened to emit. The grammar now returns the exact resolved binding spelling and retains it through character-taking commands until their operand arrives.

`App::report_completed_action` pairs that spelling with the registry description when a command has no more specific result, or with the command's concrete success status when it does. The feedback is presentation-only and tied to the status revision, so semantic and headless outcomes remain unchanged and a later LSP or other service message supersedes it. Errors, active prompts, and destructive confirmations keep their existing priority.

A follow-up fixed two precedence gaps found during independent review. Host-side failures now enter through `App::report_host_error`, which advances the status revision instead of assigning the public status fields directly, and bindings that finish by emitting a grammar error no longer receive completed-action feedback.

A later fix keeps completed-command feedback while a popup owns keyboard input. Filtering or navigating a result list is part of the command that opened it, so `Space g f` remains identified below the Git commit picker instead of falling back to the old `Space g …` prefix. Input returning to the editor clears that feedback normally, and a popup action that emits a newer status still supersedes it through the revision check.

Coverage lives in `src/app.rs`: `completed_key_bindings_report_the_typed_sequence_and_action` covers a multi-key motion and a character-taking command, `completed_actions_keep_specific_success_details` covers an explorer result and the typed `:bc` alias, `counted_colon_binding_keeps_its_error_instead_of_reporting_success` covers grammar-error precedence, and `filtering_a_git_commit_popup_keeps_the_command_that_opened_it` covers popup filtering plus the return to editor input. In `src/snapshot.rs`, `completed_binding_feedback_is_owned_by_the_snapshot_message_row` verifies that decoration reaches the frontend snapshot without changing semantic status, and `a_later_host_failure_supersedes_completed_binding_feedback` covers asynchronous host-error precedence.

Known limitation: keys owned entirely by transient overlays rather than the command registry are not labelled as completed commands.

## Report

The bottom row below the status line displayed pending command sequences such as `Space g ...`, language-server responses such as `marksman ready for markdown`, and concrete results such as `opened /path/to/dir` after `Space e`.

Many commands left the row at the pending prefix after completion instead of identifying what ran. Completed commands should show a short description together with the spelling used, for example:

```text
g l (Move to line end)
Space e (opened directory path/to/dir)
:bc (closed buffer /home/user/code/runyte)
```

This feedback should help new users understand Runyte and learn its keybindings.
