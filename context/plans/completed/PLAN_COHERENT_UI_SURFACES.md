# Runyte coherent UI surfaces

Status: completed 2026-08-17

Created: 2026-08-17

## Decision

Runyte chooses a UI surface from the lifetime and interaction of a task, not
from whichever renderer is easiest to reuse. Standalone and attached frontends
receive the same semantic state and expose the same commands.

| Surface | Use | Interaction contract |
| --- | --- | --- |
| Buffer | Durable or reusable editor content | Normal movement, selection, search, splits, copying, history, and buffer management |
| Pane-backed list | Retained rows that benefit from ordinary browsing | Buffer behavior plus row identities and scoped actions |
| Picker overlay | One immediate filtered choice | Filter, move, accept, or cancel |
| Context overlay | Assistance tied to source under the caret | Source remains active; overlay declares bounds, dismissal, and scrolling |
| Confirmation overlay | A prepared operation awaiting approval | Complete review, explicit accept/cancel, no change on cancel |
| Interaction-line prompt | One short scalar value | Enter accepts and Escape cancels |
| Input overlay | Bounded structured input | Owns input until save or cancel without pretending to be a buffer |

The canonical terminology and detailed definitions live in
`context/reference/ui-vocabulary.md`.

## Buffer and list behavior

A generated view is a buffer when its contents benefit from ordinary editor
operations or must remain available after the initiating command. Read-only
state does not make a view an overlay. Special buffers may add scoped actions,
but global commands remain available and key dispatch still comes from the
central registry.

Filterable pane-backed lists keep stable semantic row identities separate from
their rendered text. Filtering changes the projection, not the identity of the
underlying item. Opening the same durable resource reuses its buffer where the
view contract calls for reuse.

Clean detached special buffers have bounded recent-view retention. Visible or
dirty buffers are never evicted merely to satisfy that bound.

## Overlay behavior

Every overlay declares its purpose, bounds, input ownership, dismissal rules,
and actions in presentation-neutral state. A picker owns printable input while
choosing one item. A context overlay leaves the source pane active. A
confirmation represents a prepared operation and exposes no ambiguous partial
acceptance. Short scalar input alone belongs solely on the interaction line.

Overlay input is resolved before the ordinary buffer keymap only while the
overlay contract owns that input. No overlay acquires editor-like commands by
accident, and no buffer-specific action is implemented as an undocumented
renderer shortcut.

## Snapshot and protocol boundary

`src/snapshot.rs` owns presentation-neutral editor and overlay snapshots.
Frontends render those owned values without inspecting live `App` state. The
bundled local protocol mirrors bounded DTOs rather than serializing core types
directly, so transport versioning cannot turn internal editor structures into
a public compatibility promise.

Generated rows and overlays carry semantic identities and action metadata.
Frontends return typed actions to the host; they do not reconstruct editor
commands from displayed text.

## Implemented applications

The contract is used by help, settings, themes, notifications, workspace
search, Git status/history/detail views, directory buffers, filesystem-plan
review, file and symbol pickers, hover, completion, signature help, and other
contextual assistance. The Git log buffer and commit picker are the reference
pair: retained browsing uses a buffer, while an immediate fuzzy choice uses a
picker, and both open the same reusable commit-detail buffer.

## Current records

`context/reference/ui-vocabulary.md` is authoritative for terminology.
`src/snapshot.rs`, `src/picker.rs`, `src/help.rs`, and the architecture map in
`AGENTS.md` record the current implementation boundaries. This plan preserves
the decision that produced them; later source and reference updates take
precedence over historical details.
