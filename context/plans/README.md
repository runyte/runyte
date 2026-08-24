# Development plans

Plans are grouped by their relationship to the current editor:

- `active/` contains approved work that is still being designed or built.
- `proposed/` contains designs that require an explicit decision before work
  begins.
- `completed/` records implemented architecture whose rationale remains useful.
- `superseded/` contains plans whose unfinished work was replaced by a later
  decision.

Empty lifecycle directories are omitted and created only when needed. No plan
is currently active or proposed.

Completed plans are decision records, not a second user guide. Current behavior
belongs in `README.md`, `docs/user-guide.md`, the source, and the relevant file
under `context/reference/`. When those disagree with a historical plan, the
current sources take precedence.

The retained completed records cover:

- `PLAN_V4_EDITOR_CORE.md`: the rope, transaction, selection, syntax, LSP, and
  directory-buffer foundation;
- `PLAN_V8_ASYNC_WORKSPACE_GIT.md`: asynchronous services, persistent
  workspaces, the private local protocol, `--wait`, and Git workflows;
- `PLAN_COHERENT_UI_SURFACES.md`: the buffer, list, picker, prompt, and overlay
  contracts;
- `PLAN_INTEGRATED_TERMINAL.md`: host-owned terminal sessions and their
  process-lifetime persistence boundary; and
- `PLAN_PERSISTENT_SESSION_TERMINOLOGY.md`: the distinction between workspace
  scope and persistent-session lifecycle commands.

Read only the plans relevant to the part of the editor being changed.
