# Typed failure categories for notification severity

Notification severity is currently chosen at many application call sites by
calling `action_failed`, `action_warning`, or `error_from`. This classifies the
common cases correctly, but operations that return untyped strings or
`anyhow::Error` do not carry enough information for every caller to make the
same decision.

The clearest example is `App::apply_document_edits`, which returns
`Result<_, String>`. A response to an editor-initiated language-server request
reports every failure as an INFO action failure, while a server-initiated
`workspace/applyEdit` reports every failure as ERROR. Neither classification is
correct for the full error set:

- a document that changed, opened, closed, or moved since the request is a
  protective refusal and should be WARNING;
- a read-only target or an edit outside the workspace is also a protective
  refusal and should be WARNING;
- a missing target, conflicting language-server versions, an invalid range,
  or overlapping edits is malformed server output and should be ERROR; and
- failure to resolve or open a path is an external or filesystem fault and
  should be ERROR.

Other boundaries have the same underlying problem:

- the generic command-result boundary reports every propagated error as
  ERROR, including routine context refusals such as an unsupported tutorial
  argument or attempting to open the tutorial from a maximized view;
- workspace search reports filesystem traversal failures as INFO because
  `workspace_matches` errors reach `action_failed`;
- failures from the persistent-session host during the worktree-removal
  cascade can be reported as INFO even after the Git mutation has succeeded;
  and
- post-Git reconciliation reports both a dirty buffer protected from reload
  and an actual filesystem reload failure as INFO, though the former should be
  WARNING and the latter ERROR.

## Expected design

Failure classification should describe the underlying condition rather than
the notification presentation chosen by a particular caller. A small shared
semantic category is sufficient:

```rust
enum FailureClass {
    /// A normal negative outcome such as an unavailable action or no match.
    Routine,

    /// Runyte stopped an operation to protect newer, read-only, or unsafe state.
    Protective,

    /// An external command, I/O operation, protocol peer, or invariant failed.
    Fault,
}
```

The application notification boundary maps `Routine` to INFO, `Protective` to
WARNING, and `Fault` to ERROR. Interaction-line outcome styling remains a
separate decision: a rejected command can still be shown as a failed action on
the interaction line while its retained notification is INFO or WARNING.

Fallible subsystems should use operation-specific error types rather than one
application-wide enum. For example, document edit application could return a
`DocumentEditFailure` with variants such as `DocumentChanged`, `ReadOnly`,
`OutsideWorkspace`, `MissingTarget`, `ConflictingVersions`, `InvalidRange`,
`OverlappingEdits`, `ResolvePath`, and `OpenFile`. That type owns the mapping
from each variant to `FailureClass`, while its `Display` implementation keeps
the current detailed user-facing messages. Both editor-initiated and
server-initiated edit callers then pass the same typed failure through one
reporting helper, so provenance cannot change its severity.

The generic command boundary should treat unclassified `anyhow::Error` values
as `Fault` by default. Expected refusals can migrate incrementally to a small
typed error, for example `CommandRefusal::Routine` and
`CommandRefusal::Protective`, which the dispatcher recognizes without losing
the original message. The safe default must remain ERROR: a new I/O, Git,
host, or internal failure should not silently become informational merely
because its producer has not yet been classified.

Workspace traversal, host operations, and post-Git reconciliation should
likewise preserve whether an outcome is protective or a fault across helper
boundaries. Where a helper already has a concrete error type, it may expose a
`FailureClass` mapping directly; it does not need to wrap the error only to
change its notification severity.

## Constraints

- Do not determine severity by matching user-facing error strings.
- Keep notification source, title, details, and diagnostic logging intact.
- Preserve structured external-command diagnostics, including command,
  status, stdout, and stderr where currently available.
- Routine successful polling and other deliberately silent outcomes must not
  start creating notifications merely because a failure category exists.
- Avoid a single enum containing every failure in the editor. The shared type
  is the small semantic classification; operation-specific enums retain the
  useful details.
- Classification must be identical for the same failure regardless of which
  request or event path reached it.

## Regression coverage

Tests should assert retained notification severity at the behavior boundary,
not only the interaction-line message. Coverage should include:

- stale and read-only LSP edits as WARNING;
- malformed LSP ranges, overlapping edits, and file-open failures as ERROR
  through both document-edit call paths;
- routine tutorial and other command-context refusals as INFO while an
  unexpected propagated command failure remains ERROR;
- workspace-search traversal failure as ERROR;
- persistent-session host failure during worktree cleanup as ERROR; and
- dirty-buffer post-Git reconciliation as WARNING while reload I/O failure is
  ERROR.
