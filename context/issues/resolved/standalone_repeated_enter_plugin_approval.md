---
title: "Repeated Enter can approve plugin interactions in standalone mode"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: fc30f47
---

## Resolution

Commit `fc30f47` (`Preserve repeat provenance through frontend input dispatch`)
fixed the frontend dispatch gap. The standalone loop and shared host helper
previously sent repeated input through ordinary `HostCommand::Input` unless a
context overlay was open. Plugin and provider approval surfaces did not meet
that condition, so a held Enter could arrive as fresh physical input.

`WorkspaceHost::execute_frontend_input` now chooses repeat-aware dispatch for
every repeated event. The standalone and host paths share that choice while
keeping their existing hint handling and motion dispatch counts. The Windows
persistent path also uses it. A fresh unmodified Enter still approves the
current surface.

Coverage is in `src/workspace/host/tests/plugin_interaction.rs`:
`frontend_repeat_cannot_approve_confirmation_presented_after_invocation` and
`frontend_repeat_edits_plugin_form_but_waits_for_fresh_enter_to_submit`.
`frontend_repeat_keeps_provider_reload_pending_until_fresh_enter` in
`src/workspace/host/tests/plugin_recovery.rs` and
`frontend_repeat_keeps_provider_overwrite_pending_until_fresh_enter` in
`src/workspace/host/tests/plugin_provider_overwrite.rs` cover the provider
surfaces. All four focused regressions passed, along with the frontend hint
test and formatting check.

Known limitation: native Windows physical held-Enter acceptance was not run
in the Linux development environment.

## Report

The 2026-09-25 release review of `dev` at `b39bc86`, compared with `main` at
`6e6f270`, found that the standalone frontend does not consistently preserve
key-repeat provenance when dispatching plugin interactions. This is a
source-confirmed dispatch defect; a physical-key reproduction on native Windows
was not performed during the review.

In `src/main.rs::run`, the standalone input loop calls
`WorkspaceHost::execute_repeated_input` only when both `repeated` and
`context_overlay_active()` are true. A plugin input or confirmation surface
does not satisfy that overlay check. Its repeated Enter instead reaches
`HostCommand::Input` and `App::handle_input` in `src/app/input.rs`, which treats
an unmodified Enter as fresh physical input and permits approval.

`App::finish_plugin_input` in `src/app/plugin_interaction.rs` can consequently
set `native_handoff_allowed` for a submission produced by a repeated key. On
Windows this defeats the new requirement for a fresh physical Enter before a
plugin external-program or terminal handoff. Holding Enter while a plugin opens
a prompt can also submit that prompt without a separate deliberate keypress.
The same dispatch choice bypasses the repeat-aware handling of provider reload
and overwrite confirmations.

The Windows persistent-host dispatcher,
`src/main.rs::dispatch_host_repeated_key_or_text`, already forwards repeats to
the protected input boundary. The shared `dispatch_host_key_or_text` helper
retains the same context-overlay-only condition as the standalone loop and
should be audited with it.

Expected behavior is that every dispatched repeat retains its repeat identity.
Repeated editing and movement must continue to work, including configured
motion multiplication and foreground-intent invalidation, but repeated Enter
must not approve a plugin surface or acquire native handoff authority. A later
fresh unmodified Enter must still be able to approve the current surface.

## Suggested fix method

Route repeated key/text input through `execute_repeated_input` independently of
which overlay is active. Keep the existing hint handling and motion dispatch
count, and use ordinary `HostCommand::Input` for fresh input. Prefer one shared
dispatch decision for the standalone loop and applicable host paths so their
approval behavior cannot diverge. Preserve the Windows persistent path's
existing protection.

Add regression coverage at the frontend dispatch boundary, rather than calling
`App::handle_repeated_input` directly. Present a plugin confirmation, dispatch
an unmodified Enter marked as repeated, and assert that the surface remains
pending and no accepted submission or handoff authority is produced. Dispatch a
fresh Enter and verify that approval succeeds. Cover an invocation followed by
an asynchronously presented confirmation while Enter remains held, provider
reload/overwrite surfaces, and repeat-driven foreground invalidation.

Existing tests establish the lower-level contract but do not exercise the
faulty frontend branch:

- `repeated_form_editing_works_but_repeated_enter_waits_for_fresh_approval` in
  `src/app/tests/plugin_validation.rs`;
- `native_handoff_requires_current_nonreplayed_unmodified_physical_enter` in
  `src/workspace/host/tests/plugin_handoffs_windows.rs`.

Run native Windows acceptance through the real standalone frontend, including
held Enter across a prompt transition, alongside the Rust handoff gates and
Linux/macOS regression and coverage gates. Use fixture-owned configuration and
temporary storage; no personal plugin configuration or external application is
needed for the regression.
