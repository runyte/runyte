# Development plans

Plans are grouped by their relationship to the current editor:

- `active/` contains approved work that is still being designed or built.
- `proposed/` contains designs that require an explicit decision before work
  begins.
- `completed/` records implemented architecture whose rationale remains useful.
- `superseded/` contains plans whose unfinished work was replaced by a later
  decision.

Empty lifecycle directories are omitted and created only when needed.

Active work:

- [Filesystem-plan data safety](active/PLAN_FS_PLAN_DATA_SAFETY.md): atomic
  destination collision protection, safe rollback, and recoverable staging
  on Linux and macOS. The design is recorded; implementation has not started.
  Full hostile-process filesystem confinement remains deferred.

No plan is currently proposed.

Completed plans are decision records, not a second user guide. Current behavior
belongs in `README.md`, `docs/user-guide.md`, the source, and the relevant file
under `context/reference/`. When those disagree with a historical plan, the
current sources take precedence.

The retained completed records cover:

- `PLAN_KEY_REMAPPING.md`: a bounded `keys` section that remaps default
  bindings, advertised aliases, and the application and window prefixes into
  the one keymap read by dispatch and every live teaching surface;
- `PLAN_V4_EDITOR_CORE.md`: the rope, transaction, selection, syntax, LSP, and
  directory-buffer foundation;
- `PLAN_V8_ASYNC_WORKSPACE_GIT.md`: asynchronous services, persistent
  workspaces, the private local protocol, `--wait`, and Git workflows;
- `PLAN_COHERENT_UI_SURFACES.md`: the buffer, list, picker, prompt, and overlay
  contracts;
- `PLAN_INTEGRATED_TERMINAL.md`: host-owned terminal sessions and their
  process-lifetime persistence boundary;
- `PLAN_PERSISTENT_SESSION_TERMINOLOGY.md`: the distinction between workspace
  scope and persistent-session lifecycle commands; and
- `PLAN_THEME_CONSTRUCTION.md`: uniform family registration behind the shared
  theme-definition and resolution contract.

Read only the plans relevant to the part of the editor being changed.
