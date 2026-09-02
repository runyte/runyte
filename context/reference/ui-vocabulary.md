# Runyte UI vocabulary

These names are the register of record for documentation, source comments,
frontend protocol fields, and help text. Any future extension API must inherit
them, regardless of which extensibility direction is chosen.

- **Runyte screen** — the complete terminal surface owned by Runyte.
- **Startup presentation** — the stable, document-free Runyte screen shown by
  a standalone launch while its initial editor state is built. It says
  `Opening workspace…` and is replaced once by the first complete editor frame;
  it never previews unhighlighted document text or changes layout while work is
  pending.
- **Editor area** — the part of the screen above the two global lines. It
  contains every pane and overlay.
- **Pane** — one view of one buffer. Splits create panes; buffers may be shared
  by several panes. The active pane keeps the theme background; inactive panes
  use a derived ground halfway between it and the overlay ground.
- **Pane border** — the frame delimiting a pane.
- **Pane title** — structural buffer identity in the pane's top border, such as
  `[file] path`, `[explorer] path`, `[notifications]`, or `[log]`, plus `[+]`,
  `[STALE]`, and `[RO]`, in that order, and finally `[zen]` or `[fullscreen]`
  while the pane is the maximized one. `[STALE]` means an ordinary file's path
  no longer agrees with the disk baseline Runyte accepted; it is independent
  of `[+]`, which means the buffer text differs from its baseline.
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
  Active-buffer state includes `[+]`, `[STALE]`, and `[RO]` with the same
  meanings and order as the pane title.
  A long-running background action temporarily replaces the ordinary fields
  with its action name, target or query, elapsed time, optional cancellation
  hint, and a rotating spinner directly beside that text at the right edge.
  Workspace search uses `Searching workspace`; Git mutations name their
  operation. The interaction line remains independent and available for input
  and action results while this progress is visible.
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
  category. Runyte retains the eight most recently active clean special buffers
  across pane switches; activating a ninth retires the least recently used one
  once it is detached. A visible special buffer is never evicted out from
  under its pane, and a dirty one remains open and discoverable until saved or
  discarded. The editable
  explorer and commit-message buffer are special;
  pathless scratch text is not. The complete scoped set is `Directory`,
  `Settings`, `GitStatus`, `GitBranches`, `GitWorktrees`, `GitLog`, `GitBlame`,
  `GitStash`, `WorkspaceSearch`, `Help`, `CommitMessage`, and `Diff` in
  `BindingScope`. The notification buffer, the diagnostic-log buffer, and the
  about page are special too, but use the global scope because they have no
  actions of their own yet. Generated help and about pages may carry semantic
  colour spans for headings, commands, keys, paths, links, and technical
  literals. Those spans are presentation metadata over ordinary buffer
  character offsets: they add no markup characters, actions, or alternate
  coordinates.
- **Pane-backed filterable list** — a bounded-lifetime special buffer whose
  stable rows are actions or destinations. Filtering is an operation on the
  view; the list otherwise speaks normal Runyte and does not permanently own
  printable input.
- **Picker overlay** — a transient choose-one overlay. Printable input filters
  candidates, Enter accepts the selected candidate, and Escape, `Ctrl-c`, or
  an initial bare Space cancels the request. The counts in a picker's header
  — how many candidates matched, out of how many are on hand, written
  `140/570 matched` — are paced rather than live: they change at most once a
  second, whether a background scan or the reader's own typing moved them,
  and nothing releases that interval early. A header says nothing about
  whether a scan is running, and an attached client's overlay header carries
  the same counts and the same silence: one paced state, published into a
  snapshot rather than drawn. Scanner and ranker progress arrives far faster
  than a header can be read, and the counts sit inside the title, so every
  digit they gain or lose also moves the words after them. The rows are paced
  on a shorter clock of their own: a ranked answer waits until the list under
  the reader is a quarter-second old, so typing does not turn the whole list
  over on every keystroke. Pacing holds an answer back from the reader, never from
  the reader's own keys — every picker key but a query edit reads the list
  and publishes the newest answer before it runs, and a header offers `Enter`
  only where publishing what is held would leave rows that can be opened — and both clocks come due
  without an event to carry them, so an event loop asks the editor how long
  it has to wait and comes back for them. Once a project/content finder
  query contains text, Space remains its term separator. The **project
  finder** is one picker with two Tab-switched modes over the same three source
  kinds. A finder also has a **scan scope**: the root it walks and whether it
  reads the ignore files it finds. The scope belongs to the picker rather than
  to one scan, because a mode switch and a content re-scan both restart the
  walk on the reader's behalf. Three keys open the same overlay over three
  scopes — the project's ignore-aware files, every file the project holds, and
  every file under a typed path that need not be inside the workspace — so a
  finder that is not the ordinary project one names its scope in the title
  rather than looking identical to the one that is. **Name mode** merges files, open buffers, and terminal sessions by
  resource identity. **Content mode** merges file lines, authoritative
  in-memory buffer lines including pathless buffers, and decoded retained
  terminal rows. Matching characters are emphasized in the content detail
  column, including on the selected row and in attached-client snapshots. The
  preview column shows a content match as a numbered snippet around the
  matching row with the matched text highlighted, and does so for an open
  buffer or a terminal row exactly as for a file on disk. The
  query and `Ctrl-t` preview preference survive the switch; modes are not
  separate overlays or separate stops in a picker cycle. Query text and its
  caret are immediate editor-owned state; filesystem discovery, file ranking,
  live-resource matching, result merging, and disk previews advance outside
  input handling and are tagged with the query revision. The previous rows stay
  visible while a new revision ranks — a query keystroke does not blank the
  list for the length of a round trip — but cannot be accepted, and an older
  result or preview never replaces the current revision. Content search keeps
  that promise across the re-scan its own corpus forces. A content scan
  collects the lines one query matches, so a query the entries on hand cannot
  answer is re-walked rather than re-ranked; that walk waits for the query to
  stop moving, so a burst of typing costs one of them rather than one per
  character, and until it runs the rows narrow in memory against what the
  last walk collected. The corpus a walk replaces is kept for one generation
  so the rows ranked against it stay readable — a row names the scan it
  belongs to and is read through that, never through the table that replaced
  it — and while the new walk is still running an answer with no rows is
  taken as not having found any yet rather than as the query having none.
  The flush a finished scan asks for is the exception: that one is the
  answer, empty or not. A file match is
  an index into one scan's entry table and is read only together with that
  scan, so a restarted scan retires the rows ranked against the table it
  replaced rather than resolving their indices in the new one. A live terminal
  is read on a slow interval rather than on each chunk its child writes: the
  finder is a list to be read, so it holds still while a build or a test run
  scrolls, and its rows are current as of the last interval rather than of the
  last write. A refresh reads only what the child has added since, so results
  already found stay put rather than being dropped and found again, and a
  resize counts as a change because narrowing rewrites every retained line the
  finder has read. In name mode a terminal contributes its title, command, and
  activity rather than its output, so a refresh that finds the item it already
  held changes nothing and is not ranked: writing alone does not move the
  list. No frame is drawn between a refresh dropping a terminal's
  rows and finding them again: that state is a hole rather than an answer. A
  content result from a terminal is numbered by its place in the child's whole
  output, which does not change as bounded history scrolls or is evicted,
  rather than by its retained row.
- **Context overlay** — temporary information or assistance tied to the source
  under the caret, such as hover documentation, completion, or a signature.
  It leaves the source pane active and declares its own bounds and dismissal
  or scrolling behavior.
- **Confirmation overlay** — a pending prepared operation that must be fully
  inspectable before it can be accepted. It names the accept and cancel actions
  and cancellation changes nothing. Ordinary decisions accept Enter. One
  exact-text input row is reserved for a destructive operation whose target
  lacks another durable reference, or for acknowledging that an in-place
  working-tree change will continue beneath a live terminal job; Enter accepts
  only when that row matches the target-specific acknowledgment it names.
- **Interaction-line prompt** — ownership of the interaction line for one
  short scalar value until Enter accepts or Escape cancels it.
- **Input overlay** — a bounded input surface used when editing a value needs
  choices, preview, or inline validation. It owns input until save or cancel,
  but is not a buffer.
- **Completing prompt** — an interaction-line prompt whose typed value is
  matched against the filesystem as it is edited, offering its rows in a
  bounded hint list above the line. The rows are a completion of the value
  being typed, not a choose-one request, so the prompt keeps the interaction
  line and Enter still accepts what was typed rather than what is selected.
  The palette's path arguments and the finder-path prompt behind `Space / p`
  are the two of these; both spell `~`, a relative path, and a trailing
  separator the same way, and both take `Tab` as accept.

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

Modal result lists, choice overlays, action menus, and confirmations use bare
Space as a symmetric dismissal key through their existing cancellation paths.
Exact-text confirmations keep Space as literal input, because the durable path
they ask for may contain one. Interaction-line prompts are not overlays and
retain ordinary space entry.

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
  wrapped, so the ground always squares off a single rectangle. A row may
  declare one short trailing-detail column that stays pinned to the visible
  right edge while overlong identity text in its middle is clipped; the
  session manager's last-active age is the reference use.
- **Row emphasis** — the colours a row assigns to its own parts: the matched
  characters of a fuzzy query, the active parameter of a signature, an
  available command's name in the theme's `command` colour against its muted
  category, an action's mnemonic label against its muted description. It answers *what about this
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

Severity describes the condition behind the feedback, independently of
whether the requested action completed:

- `INFO` means Runyte and its dependencies are operating normally. It includes
  expected context refusals, unavailable actions, and empty results such as a
  search pattern that was not found.
- `WARNING` means an action was refused to protect data or preserve a
  consistency boundary, or it completed with a condition that needs review.
  A read-only save and a save refused because the file changed on disk are the
  reference cases.
- `ERROR` means Runyte or an external dependency failed to perform work it was
  expected to perform. Failed Git commands, host operations, file I/O, and
  language-server protocol failures are the reference cases.

An interaction-line outcome may therefore say `failed` while its retained
notification is `INFO`: the former describes the requested action, while the
latter describes system health and urgency.

Notifications enter the workspace-lifetime **notification center** without
stealing focus. `:notifications` and `:not` project the complete retained
history into the single read-only `[notifications]` buffer. The global status
line shows unacknowledged counts; opening the buffer acknowledges the entries
then retained. The center belongs to editor state, so a persistent session
host retains it across TUI detach/reattach, but it is never written to disk.
The configured history limit bounds entries; independent 1 MiB per-entry and
8 MiB per-workspace payload limits bound memory. Truncation is explicit in the
retained text. A notification buffer is materialized only while one is open.

## Diagnostic log

A **diagnostic log** is the durable local file the process that owns `App`
writes lifecycle records to. It is a fourth surface beside the interaction
line, the notification center, and the service-health report, and it answers a
different question from all three: what happened, in order, including after the
process that saw it is gone. It is neither a notification surface nor an audit
trail, and an actionable failure still reaches the person through the ordinary
surfaces whether or not a record was written.

Ownership follows editor-state ownership. A standalone editor owns
`standalone-<pid>.log`; a persistent host owns `host.log`. Both sit beneath the
resolved runtime workspace state root, normally `.runyte/`, never under
Git-tracked context. A client never appends to a host's file and never forwards
records over the local protocol.

`:log-open` projects the owning process's file into the single read-only
`[log]` buffer, following the generated-page vocabulary above: it is an
ordinary buffer with normal movement, selection, search, splits, jump history,
and buffer management, and it uses the global binding scope because it has no
row actions of its own. It is a point-in-time projection, re-read by running
the command again rather than refreshed in place, and it opens the log of the
process that holds the workspace — in persistent mode, the host's.

The `log` row of the service-health report names the owner role, the active
level, the resolved path, and any logger initialization or write failure. In
persistent mode those are host facts, so a newly attached client sees how the
process holding its workspace is actually logging rather than the flags its own
launch carried.
