# The explorer's default trash workflow has no non-destructive integration coverage

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
