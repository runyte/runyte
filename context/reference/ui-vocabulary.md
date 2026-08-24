# Runyte UI vocabulary

These names are the register of record for documentation, source comments,
frontend protocol fields, and help text. Any future extension API must inherit
them, regardless of which extensibility direction is chosen.

- **Runyte screen** — the complete terminal surface owned by Runyte.
- **Editor area** — the part of the screen above the two global lines. It
  contains every pane and overlay.
- **Pane** — one view of one buffer. Splits create panes; buffers may be shared
  by several panes. The active pane keeps the theme background; inactive panes
  use a derived ground halfway between it and the overlay ground.
- **Pane border** — the frame delimiting a pane.
- **Pane title** — structural buffer identity in the pane's top border, such as
  `[file] path`, `[explorer] path`, or `[notifications]`, plus `[+]` and `[RO]`.
- **Pane body** — the complete drawable interior of a pane border.
- **Gutter** — the left part of a pane body reserved for line numbers,
  soft-wrap continuation markers, syntax-fold markers, Git change marks, and
  its separator rule.
- **Content padding** — presentation-only blank cells between the gutter and
  an aligned generated page. Padding is not buffer text and has no buffer
  coordinates.
- **Buffer viewport** — the part of the pane body in which visible buffer rows
  are projected. Editable buffers accept changes there; read-only buffers use
  the same viewport without accepting mutations.
- **Global status line** — the first global row below the editor area. It owns
  mode, workspace directory, active-buffer state, cursor/progress, selection
  count, Git/LSP summaries, long-running action progress, and unread
  notification counts. Its leftmost mode label follows the current mode's
  caret role; the rest of the row keeps the ordinary theme background. The pane
  title, not this row, owns active-buffer identity.
- **Interaction line** — the final global row. It is reserved for an active
  prompt or the last action echo. Notifications never replace it.
- **Overlay** — a temporary surface drawn over the editor area, such as a
  picker, key hints, completion, or a confirmation. An overlay is not a pane
  and does not retarget a buffer.
- **Buffer** — an editor object that participates in normal movement,
  selection, search, splits, copying, jump history, help, and buffer management
  even when its contents are generated or read-only. Ordinary buffers remain
  until explicitly closed; clean special buffers have bounded recent-view
  lifetime.
- **Special buffer** — a buffer whose contents Runyte assembles instead of
  reading them as ordinary file text. It remains a buffer, not a menu: normal
  movement, selection, search, splits, copying, jump history, help, and buffer
  management all apply, and it may add actions for the object or rows it
  represents. Read-only state and the presence of a path do not define the
  category. Runyte retains the two most recently active clean special buffers
  across pane switches; activating a third retires the least recently used one
  once it is detached. A visible special buffer is never evicted out from
  under its pane, and a dirty one remains open and discoverable until saved or
  discarded. The editable
  explorer and commit-message buffer are special;
  pathless scratch text is not. The complete scoped set is `Directory`,
  `Settings`, `GitStatus`, `GitBranches`, `GitWorktrees`, `GitLog`, `GitBlame`,
  `GitStash`, `WorkspaceSearch`, `Help`, `CommitMessage`, and `Diff` in
  `BindingScope`. The notification buffer and about page are special too, but
  use the global scope because they have no actions of their own yet.
- **Pane-backed filterable list** — a bounded-lifetime special buffer whose
  stable rows are actions or destinations. Filtering is an operation on the
  view; the list otherwise speaks normal Runyte and does not permanently own
  printable input.
- **Picker overlay** — a transient choose-one overlay. Printable input filters
  candidates, Enter accepts the selected candidate, and Escape cancels the
  request.
- **Context overlay** — temporary information or assistance tied to the source
  under the caret, such as hover documentation, completion, or a signature.
  It leaves the source pane active and declares its own bounds and dismissal
  or scrolling behavior.
- **Confirmation overlay** — a pending prepared operation that must be fully
  inspectable before it can be accepted. It names the accept and cancel actions
  and cancellation changes nothing.
- **Interaction-line prompt** — ownership of the interaction line for one
  short scalar value until Enter accepts or Escape cancels it.
- **Input overlay** — a bounded input surface used when editing a value needs
  choices, preview, or inline validation. It owns input until save or cancel,
  but is not a buffer.

Runyte selects among these surfaces by task lifetime and interaction, not by
the renderer that is most convenient or by whether the content looks like
rows in a rectangle. Navigable results belong in buffers; immediate
choose-one requests belong in pickers; source-tied assistance belongs in
context overlays; pending operations belong in confirmations; and only short
scalar input belongs solely on the interaction line.

`Space g l` and `Space g /` are the reference pair. `Space g l` opens a
retained Git-log special buffer because commit history benefits from ordinary
browsing, search, and splits. `Space g /` opens a picker because fuzzy typing
resolves one immediate commit choice. Enter from either surface opens the same
retained commit-detail special buffer.

## Notifications

A **notification** is retained feedback that should remain inspectable after
the action or service event that created it. Runyte assigns `ERROR`, `WARNING`,
or `INFO` severity at the producer boundary. Assigning a severity does not by
itself mean a successful operation should create a notification; silent
successes and routine polling remain silent.

Notifications enter the workspace-lifetime **notification center** without
stealing focus. `:notifications` and `:not` project the complete retained
history into the single read-only `[notifications]` buffer. The global status
line shows unacknowledged counts; opening the buffer acknowledges the entries
then retained. The center belongs to editor state, so a persistent session
host retains it across TUI detach/reattach, but it is never written to disk.
The configured history limit bounds entries; independent 1 MiB per-entry and
8 MiB per-workspace payload limits bound memory. Truncation is explicit in the
retained text. A notification buffer is materialized only while one is open.
