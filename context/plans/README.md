# Development plans

Plans are grouped by their relationship to the current editor:

- `active/` contains approved work that is still being designed or built.
- `proposed/` contains designs that require an explicit decision before work
  begins.
- `completed/` records implemented architecture whose rationale remains useful.
- `superseded/` contains plans whose unfinished work was replaced by a later
  decision.

Empty lifecycle directories are omitted and created only when needed.

Completed plans are decision records, not a second user guide. Current behavior
belongs in `README.md`, `docs/user-guide.md`, the source, and the relevant file
under `context/reference/`. When those disagree with a historical plan, the
current sources take precedence.

The retained completed records cover:

- [Database viewer UX and full-value inspection](completed/PLAN_DBVIEWER_UX_AND_FULL_VALUES.md):
  grouped actions, explicit view identity, labelled top metadata and on-demand
  complete-value documents through compatible `runyte-1` extensions;
- [Database viewer interactive browsing](completed/PLAN_DBVIEWER_INTERACTIVE_BROWSING.md):
  connection forms, contextual navigation, access modes, query guidance,
  interactive filters, JSON inspection and transaction management for ru-dbviewer;
- [Stable plugin compatibility](completed/PLAN_STABLE_PLUGINS.md): release-range
  declarations, a stable application contract, a clean experimental cutover and
  reproducible compatibility gates using ru-time as an external plugin;

- [Agent workspace context](completed/PLAN_AGENT_WORKSPACE_CONTEXT.md): bounded
  context reads, separately granted buffer edits, and individually approved
  terminal text proposals across authorized live workspaces. Terminal insertion
  does not submit commands.

- [Plugin applications](completed/PLAN_PLUGIN_APPLICATIONS.md): an experimental
  epoch 2 API for native application views, explicit editor operations, remote
  documents, asynchronous jobs and managed backends, with Linux performance
  acceptance and Linux/macOS CI validation;
- [Experimental process plugins](completed/PLAN_MINIMAL_PLUGINS.md): captured
  invoking context, namespaced runtime commands, revision-checked selected-text
  transactions, bounded observations, host ownership and explicit enablement;

- [Document readiness before syntax parsing](completed/PLAN_ASYNC_INITIAL_SYNTAX.md):
  deferred initial/full parsing, bounded background work, edits before a tree is
  available, command readiness, worker-owned retirement, and non-blocking quit;
- [Session and open-destination navigation](completed/PLAN_SESSION_NAVIGATION.md):
  the persistent-session strip, mixed Navigator, directory opening, parent-terminal
  handoffs and external-editor waits, and remote destination inventories;
- [Filesystem-plan data safety](completed/PLAN_FS_PLAN_DATA_SAFETY.md): atomic
  destination collision protection, safe rollback, and recoverable staging,
  validated on native Linux and macOS. Full hostile-process filesystem
  confinement remains deferred;
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
