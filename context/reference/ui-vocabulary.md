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
  `[file] path`, `[explorer] path`, or `[notifications]`, plus `[+]` and `[RO]`,
  and finally `[zen]` or `[fullscreen]` while the pane is the maximized one.
  The first markers describe the buffer; the last describes how this pane is
  presented, and is absent in an ordinary layout.
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

## Row selection

Every surface that draws a list of rows — picker overlay, pane-backed
filterable list, command palette, contextual action menu, completion — says
which row is selected the same way. The vocabulary is three separable
signals, and each one answers a different question.

- **Selection marker** — `▸ ` in the accent colour, in a two-column
  **selection gutter** at the start of the row. Every row of a
  marker-using list pays for the gutter whether or not it is selected, so a
  label never shifts sideways as the selection moves. It answers *which row*
  at a glance, and it survives a theme whose selection ground is low in
  contrast.
- **Selection ground** — the `selection` colour, run across the row from the
  gutter to the far edge of the surface, including the detail column and the
  empty space past the last character. It answers *how far the row reaches*.
  A row is one line: content too wide for the surface is truncated, never
  wrapped, so the ground always squares off a single rectangle.
- **Row emphasis** — the colours a row assigns to its own parts: the matched
  characters of a fuzzy query, the active parameter of a signature, an
  available command's accented name against its muted category, an action's
  mnemonic label against its muted description. It answers *what about this
  row*, and the other two signals never repaint it. The selection therefore
  contributes a background and no foreground, so a row's columns still read
  as columns while it is selected.

Two consequences follow from the last point and are easy to get wrong.
Ratatui applies a list's highlight style over the finished row as a patch, so
any foreground set there silently erases emphasis underneath it; give the
highlight a background only. And a row dimmed for dormancy is exempt from
dimming while it is selected, which has to be written out where the row's
colours are chosen, because the selection ground no longer repaints it back
to legibility on its own.

The contextual action menu reads across four columns, padded to the widest
entry in the open menu so they line up down it: the mnemonic, one lower-case
word naming the action, the scope it acts on (`row` or `buffer`), and the
sentence explaining it. The mnemonic is the row label, drawn in the
foreground colour; the other three are the muted detail. Only the widths
follow the actions on offer. The scope column is drawn even where every
action in a menu shares one scope, because the four columns have to mean the
same thing in every view that opens the menu. And because a truncated column
has stopped lining up with anything, the menu takes the width its widest row
needs rather than a fixed share of the editor area.

Caret-anchored context overlays — completion, signature, hover — are the one
deliberate exception, and they omit only the marker. They are narrow by
design and sit against the source text they describe, where two borrowed
columns cost more than the marker adds; the selection ground alone says which
candidate is selected.

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
