---
title: "The explorer's default trash workflow has no non-destructive integration coverage"
status: resolved
reported: 2026-08-30
resolved: 2026-08-30
commit: d870473
---

## Resolution

Commit d870473 (`Harden cross-platform integration test boundaries`) moved
trash deletion behind the `TrashBackend` boundary in `src/fs_plan.rs`.
`SystemTrash` remains the production default and delegates to the native
`trash` crate, while `FsPlan::apply_with_trash` lets an owning application or
test supply a different backend. `HostPorts` in `src/app.rs` owns that backend,
and the confirmed Explorer plan passes it only for `DeletionMode::Trash`;
permanent deletion continues through its distinct filesystem path.

The Explorer integration fixture supplies a temporary rename-based trash.
It drives the real `Ctrl-s` confirmation workflow, proves Escape leaves the
source untouched, proves Enter moves the expected contents only after
confirmation, and verifies that a backend failure preserves both the source
and the dirty Explorer edits while publishing the error.

Coverage is provided by
`enter_uses_the_injected_trash_backend_only_after_confirmation`,
`trash_backend_failure_preserves_the_source_and_explorer_edits`, and
`writing_only_opens_confirmation_and_enter_applies_the_plan` in
`tests/directory_buffer.rs`; and `permanent_deletion_is_an_explicit_apply_mode`
in `tests/fs_plan.rs`.

Known limitation: CI deliberately does not exercise a person's platform trash;
it verifies the default workflow through the injected boundary while native
trash behavior remains the responsibility of the `trash` crate.

## Report

The editable directory explorer confirms a filesystem plan and applies it
with trash deletion on `Enter`; `P` applies the same plan with permanent
deletion. The integration fixture in `tests/directory_buffer.rs` uses `P` so
that CI does not depend on Finder or on access to the person's platform trash
directory.

That keeps the test isolated, but it leaves the default `Enter` path without
behavioral coverage. A regression could route `Enter` to permanent deletion,
pass the wrong `DeletionMode` into `FsPlan::apply`, or mishandle an error from
the trash backend while the permanent-deletion fixture continues to pass.

The trash boundary should be injectable so a test can provide a temporary,
non-destructive backend. Coverage should edit an explorer to remove a file,
open the confirmation overlay, press `Enter`, and verify that:

- the plan selects `DeletionMode::Trash` rather than permanent deletion;
- the source disappears only after confirmation;
- the injected trash destination receives the expected file and contents;
- cancellation leaves the source untouched; and
- a trash-backend failure is reported while preserving the source and the
  editor's consistent explorer state.

Integration tests must never use the person's real trash, configuration, or
platform cache. They must use temporary directories and must not execute a
file they create. The production trash implementation and its platform-native
behavior should remain behind the injected boundary rather than being
reimplemented in the test.
