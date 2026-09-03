# Development plans

Plans are grouped by their relationship to the current editor:

- `active/` contains approved work that is still being designed or built.
- `proposed/` contains designs that require an explicit decision before work
  begins.
- `completed/` records implemented architecture whose rationale remains useful.
- `superseded/` contains plans whose unfinished work was replaced by a later
  decision.

Empty lifecycle directories are omitted and created only when needed. No plan
is currently active. One plan is proposed:

- `proposed/PLAN_KEY_REMAPPING.md`: a `keys` section in `config.yaml` that
  remaps the `Space` and `Ctrl-w` namespaces, the short aliases they advertise,
  and the two named prefixes, resolved once into the keymap that dispatch,
  help, hints, the manual, and the tutorial all read. Answers
  `context/issues/configurable_key_bindings.md`.

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
  process-lifetime persistence boundary;
- `PLAN_PERSISTENT_SESSION_TERMINOLOGY.md`: the distinction between workspace
  scope and persistent-session lifecycle commands; and
- `PLAN_THEME_CONSTRUCTION.md`: uniform family registration behind the shared
  theme-definition and resolution contract.

Read only the plans relevant to the part of the editor being changed.
